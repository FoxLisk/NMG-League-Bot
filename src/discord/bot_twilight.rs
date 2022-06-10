use crate::constants::{APPLICATION_ID_VAR, TOKEN_VAR};
use crate::db::get_pool;
use crate::discord::bot_twilight::state::State;
use crate::discord::{ADMIN_ROLE_NAME, CUSTOM_ID_START_RUN};
use crate::models::race::RaceState::CREATED;
use crate::models::race::{NewRace, Race};
use crate::models::race_run::RaceRun;
use crate::Shutdown;
use core::default::Default;
use dashmap::DashMap;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio_stream::StreamExt;
use twilight_cache_inmemory::InMemoryCache;
use twilight_gateway::cluster::ShardScheme;
use twilight_gateway::Cluster;
use twilight_http::client::InteractionClient;
use twilight_http::response::DeserializeBodyError;
use twilight_http::Client;
use twilight_mention::Mention;
use twilight_model::application::component::button::ButtonStyle;
use twilight_model::application::component::{ActionRow, Button, Component};
use twilight_model::application::interaction::application_command::{
    CommandDataOption, CommandOptionValue,
};
use twilight_model::application::interaction::{ApplicationCommand, Interaction};
use twilight_model::channel::message::allowed_mentions::AllowedMentionsBuilder;
use twilight_model::channel::Message;
use twilight_model::gateway::event::Event;
use twilight_model::gateway::payload::incoming::InteractionCreate;
use twilight_model::gateway::Intents;
use twilight_model::guild::Role;
use twilight_model::http::interaction::InteractionResponseType::DeferredChannelMessageWithSource;
use twilight_model::http::interaction::{
    InteractionResponse, InteractionResponseData, InteractionResponseType,
};
use twilight_model::id::marker::{ApplicationMarker, ChannelMarker, GuildMarker, UserMarker};
use twilight_model::id::Id;
use twilight_validate::component::button;

use super::CREATE_RACE_CMD;

mod state {
    use dashmap::DashMap;
    use sqlx::SqlitePool;
    use twilight_cache_inmemory::InMemoryCache;
    use twilight_http::client::InteractionClient;
    use twilight_http::Client;
    use twilight_model::id::marker::{ApplicationMarker, ChannelMarker, UserMarker};
    use twilight_model::id::Id;

    pub(super) struct State {
        pub cache: InMemoryCache,
        pub client: Client,
        pub pool: SqlitePool,
        // this isn't handled by the cache b/c it is not updated via Gateway events
        private_channels: DashMap<Id<UserMarker>, Id<ChannelMarker>>,
        aid: Id<ApplicationMarker>,
    }

    impl State {
        pub(super) fn new(
            cache: InMemoryCache,
            client: Client,
            aid: Id<ApplicationMarker>,
            pool: SqlitePool,
        ) -> Self {
            Self {
                cache,
                client,
                pool,
                aid,
                private_channels: Default::default(),
            }
        }

        pub(super) fn interaction_client<'a>(&'a self) -> InteractionClient<'a> {
            self.client.interaction(self.aid.clone())
        }

        pub(super) async fn get_private_channel(
            &self,
            user: Id<UserMarker>,
        ) -> Result<Id<ChannelMarker>, String> {
            if let Some(id) = self.private_channels.get(&user) {
                return Ok(id.clone());
            }

            let created = self
                .client
                .create_private_channel(user.clone())
                .exec()
                .await
                .map_err(|e| e.to_string())?;
            if created.status().is_success() {
                let chan = created.model().await.map_err(|e| e.to_string())?;
                self.private_channels.insert(user, chan.id.clone());
                Ok(chan.id)
            } else {
                Err(format!(
                    "Error result creating private channel: {}",
                    created.status()
                ))
            }
        }
    }
}

pub(crate) async fn launch(mut shutdown: tokio::sync::broadcast::Receiver<Shutdown>) {
    let token = std::env::var(TOKEN_VAR).unwrap();
    let aid_str = std::env::var(APPLICATION_ID_VAR).unwrap();
    let aid = Id::<ApplicationMarker>::new(aid_str.parse::<u64>().unwrap());
    let pool = get_pool().await.unwrap();

    let intents = Intents::GUILDS
        | Intents::GUILD_MESSAGES
        | Intents::GUILD_MESSAGE_REACTIONS
        | Intents::DIRECT_MESSAGES
        | Intents::DIRECT_MESSAGE_REACTIONS
        | Intents::MESSAGE_CONTENT
        | Intents::GUILD_MEMBERS
        // adding guild presences *solely* to get guild members populated in the cache to avoid
        // subsequent http requests
        | Intents::GUILD_PRESENCES;

    let (cluster, mut events) = Cluster::builder(token.clone(), intents)
        .build()
        .await
        .unwrap();

    let cluster = Arc::new(cluster);

    // Start up the cluster
    let cluster_spawn = cluster.clone();

    tokio::spawn(async move {
        cluster_spawn.up().await;
    });

    let http = Client::new(token);

    let cache = InMemoryCache::builder().build();

    let state = Arc::new(State::new(cache, http, aid, pool));

    loop {
        tokio::select! {
            Some((_shard_id, event)) = events.next() => {
                 // Update the cache.
                state.cache.update(&event);

                // Spawn a new task to handle the event
                tokio::spawn(handle_event(event, state.clone()));
            },
            _sd = shutdown.recv() => {
                println!("Twilight bot shutting down...");
                break;
            }
        }
    }
}

fn get_admin_role(guild_id: Id<GuildMarker>, state: &Arc<State>) -> Option<Role> {
    let roles = state.cache.guild_roles(guild_id)?;

    for role_id in roles.value() {
        if let Some(role) = state.cache.role(*role_id) {
            if role.name == ADMIN_ROLE_NAME {
                // cloning pulls it out of the reference, unlocking the cache
                return Some(role.resource().clone());
            }
        }
    }
    None
}

fn can_create_race(
    guild_id: Id<GuildMarker>,
    user_id: Id<UserMarker>,
    state: &Arc<State>,
) -> Result<bool, String> {
    let role =
        get_admin_role(guild_id, state).ok_or("Error: Cannot find admin role".to_string())?;
    let member = state
        .cache
        .member(guild_id, user_id)
        .ok_or("Error: cannot find member".to_string())?
        .value()
        .clone();
    Ok(member.roles().contains(&role.id))
}

/// Creates a basic interaction response: plain content with no allowed mentions.
fn plain_interaction_response<S: Into<String>>(content: S) -> InteractionResponse {
    InteractionResponse {
        kind: InteractionResponseType::ChannelMessageWithSource,
        data: Some(InteractionResponseData {
            content: Some(content.into()),
            allowed_mentions: Some(AllowedMentionsBuilder::new().build()),
            ..Default::default()
        }),
    }
}

fn button_component<S1: Into<String>, S2: Into<String>>(
    label: S1,
    custom_id: S2,
    style: ButtonStyle,
) -> Component {
    Component::Button(Button {
        custom_id: Some(custom_id.into()),
        disabled: false,
        emoji: None,
        label: Some(label.into()),
        style,
        url: None,
    })
}

async fn notify_racer(
    mut race_run: RaceRun,
    race: &Race,
    state: &Arc<State>,
) -> Result<(), String> {
    let uid = race_run.racer_id_tw()?;
    if Some(uid) == state.cache.current_user().map(|cu| cu.id) {
        println!("Not sending messages to myself");
        return Ok(());
    }
    let dm = state.get_private_channel(uid).await?;
    let content = format!(
        "Hello, your asynchronous race is now ready.
When you're ready to begin your race, click \"Start run\" and you will be given
filenames to enter.

If anything goes wrong, tell an admin there was an issue with race `{}`",
        race.uuid
    );

    let resp = state
        .client
        .create_message(dm)
        .components(&[Component::ActionRow(ActionRow {
            components: vec![button_component(
                "Start run",
                CUSTOM_ID_START_RUN,
                ButtonStyle::Primary,
            )],
        })])
        .and_then(|cm| cm.content(&content))
        .map_err(|e| e.to_string())?
        .exec()
        .await
        .map_err(|e| e.to_string())?;

    if resp.status().is_success() {
        let msg = resp.model().await.map_err(|e| e.to_string())?;
        race_run.set_message_id(msg.id.get());
        race_run.save(&state.pool).await
    } else {
        Err(format!("Error sending message: {}", resp.status()))
    }
}

async fn handle_create_race(
    mut ac: Box<ApplicationCommand>,
    state: &Arc<State>,
) -> Result<InteractionResponse, String> {
    let gid = ac
        .guild_id
        .ok_or("Create race called outside of guild context".to_string())?;
    let uid = ac
        .author_id()
        .ok_or("Create race called by no one ????".to_string())?;

    if !can_create_race(gid, uid, &state)? {
        return Err("Unprivileged user attempted to create race".to_string());
    }

    if ac.data.options.len() != 2 {
        return Err(format!(
            "Expected exactly 2 options for {}",
            CREATE_RACE_CMD
        ));
    }

    fn get_user(cdos: &mut Vec<CommandDataOption>) -> Option<Id<UserMarker>> {
        let opt = cdos.pop()?;
        match opt.value {
            CommandOptionValue::User(uid) => Some(uid),
            _ => None,
        }
    }

    let p1 = get_user(&mut ac.data.options).ok_or("Expected two users".to_string())?;
    let p2 = get_user(&mut ac.data.options).ok_or("Expected two users".to_string())?;
    if p1 == p2 {
        return Ok(plain_interaction_response(
            "The racers must be different users",
        ));
    }

    let new_race = NewRace::new();
    let race = new_race
        .save(&state.pool)
        .await
        .map_err(|e| format!("Error saving race: {}", e))?;
    let (r1, r2) = race
        .select_racers(p1, p2, &state.pool)
        .await
        .map_err(|e| format!("Error saving race: {}", e))?;

    let (first, second) = {
        tokio::join!(
            notify_racer(r1, &race, &state),
            notify_racer(r2, &race, &state)
        )
    };
    if let Err(err) = first.and(second) {
        return Ok(plain_interaction_response(format!(
            "Internal error creating race: {}",
            err
        )));
    }
    Ok(plain_interaction_response(format!(
        "Race created for users {} and {}",
        p1.mention(),
        p2.mention(),
    )))
}

async fn handle_interaction(interaction: InteractionCreate, state: Arc<State>) {
    let interaction_id = interaction.id();
    let token = interaction.token().to_string();
    match interaction.0 {
        Interaction::ApplicationCommand(ac) => match ac.data.name.as_str() {
            CREATE_RACE_CMD => {
                let resp = match handle_create_race(ac, &state).await {
                    Ok(r) => r,
                    Err(e) => {
                        println!("Error creating race: {}", e);
                        InteractionResponse {
                            kind: InteractionResponseType::ChannelMessageWithSource,
                            data: Some(InteractionResponseData {
                                content: Some("Internal error creating race".to_string()),
                                ..Default::default()
                            }),
                        }
                    }
                };

                if let Err(e) = state
                    .interaction_client()
                    .create_response(interaction_id, &token, &resp)
                    .exec()
                    .await
                {
                    println!("Error creating interaction: {}", e);
                }
            }
            _ => {
                println!(
                    "Twilight bot: Unhandled ApplicationCommand: {}",
                    ac.data.name
                );
            }
        },
        _ => {}
    }
}

async fn handle_event(event: Event, state: Arc<State>) {
    match event {
        Event::GuildCreate(gc) => {
            println!("Guild create event: {:?}", gc);
        }
        Event::GuildDelete(gd) => {
            println!("Guild deleted?? event: {:?}", gd);
        }

        Event::GuildUpdate(gu) => {
            println!("Guild updated event: {:?}", gu);
        }
        Event::InteractionCreate(ic) => {
            handle_interaction(ic, state).await;
        }
        Event::Ready(r) => {
            println!("Ready! {:?}", r);
        }
        Event::RoleDelete(rd) => {
            println!("Role deleted: {:?}", rd);
        }
        Event::RoleUpdate(ru) => {
            println!("Role updated: {:?}", ru);
        }
        _ => {}
    }
}
