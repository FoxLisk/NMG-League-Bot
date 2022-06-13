use crate::constants::{APPLICATION_ID_VAR, TOKEN_VAR};
use crate::db::get_pool;
use crate::discord::bot_twilight::state::State;
use crate::discord::{ADMIN_ROLE_NAME, CUSTOM_ID_FINISH_RUN, CUSTOM_ID_FORFEIT_RUN, CUSTOM_ID_START_RUN, CUSTOM_ID_USER_TIME, CUSTOM_ID_USER_TIME_MODAL, CUSTOM_ID_VOD_READY};
use crate::models::race::RaceState::CREATED;
use crate::models::race::{NewRace, Race};
use crate::models::race_run::RaceRun;
use crate::{Shutdown, Webhooks};
use core::default::Default;
use dashmap::DashMap;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::future::Future;
use std::ops::Deref;
use std::sync::Arc;
use serenity::model::interactions::message_component::ComponentType;
use tokio_stream::StreamExt;
use twilight_cache_inmemory::InMemoryCache;
use twilight_gateway::cluster::ShardScheme;
use twilight_gateway::Cluster;
use twilight_http::client::InteractionClient;
use twilight_http::response::DeserializeBodyError;
use twilight_http::Client;
use twilight_mention::Mention;
use twilight_model::application::command::{
    BaseCommandOptionData, Command, CommandOption, CommandType,
};
use twilight_model::application::component::button::ButtonStyle;
use twilight_model::application::component::{ActionRow, Button, Component, TextInput};
use twilight_model::application::component::text_input::TextInputStyle;
use twilight_model::application::interaction::application_command::{
    CommandDataOption, CommandOptionValue,
};
use twilight_model::application::interaction::{ApplicationCommand, Interaction, MessageComponentInteraction};
use twilight_model::application::interaction::modal::{ModalInteractionDataActionRow, ModalInteractionDataComponent, ModalSubmitInteraction};
use twilight_model::channel::message::allowed_mentions::AllowedMentionsBuilder;
use twilight_model::channel::Message;
use twilight_model::gateway::event::Event;
use twilight_model::gateway::payload::incoming::{GuildCreate, InteractionCreate};
use twilight_model::gateway::Intents;
use twilight_model::guild::{Guild, Permissions, Role};
use twilight_model::http::interaction::InteractionResponseType::DeferredChannelMessageWithSource;
use twilight_model::http::interaction::{
    InteractionResponse, InteractionResponseData, InteractionResponseType,
};
use twilight_model::id::marker::{ApplicationMarker, ChannelMarker, GuildMarker, MessageMarker, UserMarker};
use twilight_model::id::Id;
use twilight_util::builder::command::CommandBuilder;
use twilight_util::builder::InteractionResponseDataBuilder;
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
    use crate::Webhooks;

    pub(super) struct State {
        pub cache: InMemoryCache,
        pub client: Client,
        pub pool: SqlitePool,
        pub webhooks: Webhooks,
        // this isn't handled by the cache b/c it is not updated via Gateway events
        private_channels: DashMap<Id<UserMarker>, Id<ChannelMarker>>,
        application_id: Id<ApplicationMarker>,

    }

    impl State {
        pub(super) fn new(
            cache: InMemoryCache,
            client: Client,
            aid: Id<ApplicationMarker>,
            pool: SqlitePool,
            webhooks: Webhooks,
        ) -> Self {
            Self {
                cache,
                client,
                pool,
                webhooks,
                application_id: aid,
                private_channels: Default::default(),
            }
        }

        pub(super) fn interaction_client<'a>(&'a self) -> InteractionClient<'a> {
            self.client.interaction(self.application_id.clone())
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

pub(crate) async fn launch(webhooks: Webhooks, mut shutdown: tokio::sync::broadcast::Receiver<Shutdown>) {
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
    let cluster_start = cluster.clone();
    let cluster_stop = cluster.clone();
    tokio::spawn(async move {
        cluster_start.up().await;
    });

    let http = Client::new(token);
    let cache = InMemoryCache::builder().build();
    let state = Arc::new(State::new(cache, http, aid, pool, webhooks));

    loop {
        tokio::select! {
            Some((_shard_id, event)) = events.next() => {
                state.cache.update(&event);
                tokio::spawn(handle_event(event, state.clone()));
            },
            _sd = shutdown.recv() => {
                println!("Twilight bot shutting down...");
                cluster_stop.down();
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

/// InteractionResponseData with just content + no allowed mentions
fn plain_interaction_data<S: Into<String>>(content: S) -> InteractionResponseData {
    InteractionResponseData {
        content: Some(content.into()),
        allowed_mentions: Some(AllowedMentionsBuilder::new().build()),
        ..Default::default()
    }
}

/// Creates a basic interaction response: new message, plain content with no allowed mentions.
fn plain_interaction_response<S: Into<String>>(content: S) -> InteractionResponse {
    InteractionResponse {
        kind: InteractionResponseType::ChannelMessageWithSource,
        data: Some(plain_interaction_data(content)),
    }
}

fn update_resp_to_plain_content<S: Into<String>>(content: S) -> InteractionResponse {
    InteractionResponse {
        kind: InteractionResponseType::UpdateMessage,
        data: Some(InteractionResponseData {
            components: Some(vec![]),
            content: Some(content.into()),
            ..Default::default()
        })
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

async fn handle_run_start(interaction: Box<MessageComponentInteraction>, state: &Arc<State>) -> Result<(), String> {
    let mut rr = RaceRun::search_by_message_id_tw(&interaction.message.id, &state.pool).await?
        .ok_or(format!("No RaceRun found for message id {}", interaction.message.id))?;
    rr.start();
    match rr.save(&state.pool).await {
        Ok(_) => {
            let buttons = vec![Component::ActionRow(ActionRow {
                components: vec![button_component(
                    "Finish run",
                    CUSTOM_ID_FINISH_RUN,
                    ButtonStyle::Success,
                ), button_component(
                    "Forfeit",
                    CUSTOM_ID_FORFEIT_RUN,
                    ButtonStyle::Danger
                )],
            })];
            let content = format!("Good luck! your filenames are: `{}`

If anything goes wrong, tell an admin there was an issue with race `254bb3a6-5d23-4198-80bb-40f9994298c9`
", rr.filenames().unwrap());
            let resp = InteractionResponse {
                kind: InteractionResponseType::UpdateMessage,
                data: Some(InteractionResponseDataBuilder::new()
                    .components(buttons)
                    .content(content)
                    .build())
            };
            // TODO: why is this creating a new response instead of updating the response?
            state.interaction_client().create_response(
                interaction.id,
                &interaction.token,
                &resp
            ).exec()
                .await
                .map_err(|e| e.to_string())
                .map(|_|())
        }
        Err(e) => {
            println!("Error updating race run: {}", e);
            let ir = update_resp_to_plain_content("There was an error starting your race. Please ping FoxLisk.");

            state.interaction_client().create_response(
                interaction.id,
                &interaction.token,
                &ir
            ).exec().await.map_err(|e| e.to_string()).map(|_|())
        }
    }
}


trait GetMessageId {
    fn get_message_id(&self) -> Option<Id<MessageMarker>>;
}

impl GetMessageId for MessageComponentInteraction {
    fn get_message_id(&self) -> Option<Id<MessageMarker>> {
        Some(self.message.id)
    }
}

impl GetMessageId for ModalSubmitInteraction {
    fn get_message_id(&self) -> Option<Id<MessageMarker>> {
        self.message.as_ref().map(|m| m.id.clone())
    }
}

async fn update_race_run<M, T, F>(interaction: &T, f: F, state: &Arc<State>) -> Option<String>
    where
        M: GetMessageId,
        T: Deref<Target=M> + Debug,
        F: FnOnce(&mut RaceRun) -> ()
{
    let mid = match interaction.get_message_id() {
        Some(id) => id,
        None => { return Some(format!("Interaction {:?} has no message id", interaction)); }
    };

    let rro = match RaceRun::search_by_message_id_tw(&mid, &state.pool).await {
        Ok(r) => {r}
        Err(e) => {
            return Some(e);
        }
    };
    match rro {
        Some(mut rr) => {
            f(&mut rr);
            {
                if let Err(e) = rr.save(&state.pool).await {
                    Some(format!("Error saving race {}: {}", rr.id, e))
                } else {
                    None
                }
            }
        }
        None => {
            Some(format!("Update for unknown race with message id {}", mid))
        }
    }
}

async fn handle_run_forfeit(interaction: Box<MessageComponentInteraction>, state: &Arc<State>) -> Result<(), String> {
    let content = if let Some(e) = update_race_run(&interaction, |rr| {rr.forfeit();}, state ).await {
        println!("Error forfeiting race: {}", e);
        state.webhooks.message_async(&format!(
            "{} tried to forfeit a race but encountered an internal error: {}",
            interaction.author_id().map(|m| m.mention().to_string())
                .unwrap_or("Unknown user".to_string()),
            e)).await.ok();
            "Something went wrong forfeiting this match. Please ping FoxLisk."
    } else{
            "You have forfeited this match. Please let the admins know if there are any issues."
    };
    let ir = update_resp_to_plain_content(content);

    state.interaction_client().create_response(
        interaction.id,
        &interaction.token,
        &ir
    ).exec().await.map_err(|e| e.to_string()).map(|_|())
}


async fn handle_run_finish(interaction: Box<MessageComponentInteraction>, state: &Arc<State>) -> Result<(), String> {
    if let Some(e) = update_race_run(&interaction, |rr| {rr.finish();}, state ).await {
        println!("Error finishing race: {}", e);
        state.webhooks.message_async(&format!(
            "{} tried to finish a race but encountered an internal error: {}",
            interaction.author_id().map(|m| m.mention().to_string())
                .unwrap_or("Unknown user".to_string()),
            e)).await.ok();

        let ir = update_resp_to_plain_content(
            "Something went wrong finishing this match. Please ping FoxLisk.");

        return state.interaction_client().create_response(
            interaction.id,
            &interaction.token,
            &ir
        ).exec().await.map_err(|e| e.to_string()).map(|_|());
    }
    let ir = InteractionResponse {
        kind: InteractionResponseType::Modal,
        data: Some(InteractionResponseData {
            components: Some(vec![
                Component::ActionRow(ActionRow {
                    components: vec![Component::TextInput(TextInput{
                            custom_id: CUSTOM_ID_USER_TIME.to_string(),
                            label: "Finish time:".to_string(),
                            max_length: Some(100),
                            min_length: Some(5),
                            placeholder: None,
                            required: Some(true),
                            style: TextInputStyle::Short,
                            value: None
                        })],
                })
            ]),
            content: Some("Please enter finish time in **H:MM:SS** format".to_string()),
            custom_id: Some(CUSTOM_ID_USER_TIME_MODAL.to_string()),
            title: Some("Enter finish time in **H:MM:SS** format".to_string()),
            ..Default::default()
        })
    };

    state.interaction_client().create_response(
        interaction.id,
        &interaction.token,
        &ir
    ).exec().await.map_err(|e| e.to_string()).map(|_|())
}

async fn handle_button_interaction(interaction: Box<MessageComponentInteraction>, state: &Arc<State>) -> Result<(), String> {
    match interaction.data.custom_id.as_str() {
        CUSTOM_ID_START_RUN => handle_run_start(interaction, state).await,
        CUSTOM_ID_FORFEIT_RUN => handle_run_forfeit(interaction, state).await,
        CUSTOM_ID_FINISH_RUN => handle_run_finish(interaction, state).await,
        _ => {
            println!("Unhandled button: {:?}", interaction);
            Ok(())
        }
    }
}

fn get_field_from_modal_components(rows: Vec<ModalInteractionDataActionRow>, custom_id: &str) -> Option<String> {
    for row in rows {
        for cmp in row.components {
            // modal interaction components can be ActionRows, but they can't have sub-components?
            // I don't really get what's going on here, but I think a modal is basically just
            // an action row + some text inputs. that's all I'm doing, anyway, so it's fine
            println!("component custom id: {} looking for {}", cmp.custom_id, custom_id);
            if cmp.custom_id == custom_id {
                return Some(cmp.value);
            }
        }
    }
    None
}

async fn handle_user_time_modal(mut interaction: Box<ModalSubmitInteraction>, state: &Arc<State>) -> Result<(), String> {
    let mid = interaction.get_message_id().ok_or("No message found for interaction???".to_string())?;
    let rr = match RaceRun::get_by_message_id_tw(&mid, &state.pool).await {
        Ok(r) => {
            r
        }
        Err(e) => {
            println!("Error finding race run: {}", e);
            let ir = update_resp_to_plain_content(
                "Something went wrong reporting your time. Please ping FoxLisk.");
            return state.interaction_client().create_response(
                interaction.id,
                &interaction.token,
                &ir
            ).exec().await.map_err(|e| e.to_string()).map(|_|());
        }
    };

    let ut = match get_field_from_modal_components(std::mem::take(&mut interaction.data.components), CUSTOM_ID_USER_TIME) {
        Some(s) => s,
        None => {
            println!("Error getting user time from modal");
            let ir = update_resp_to_plain_content(
                "Something went wrong reporting your time. Please ping FoxLisk.");
            return state.interaction_client().create_response(
                interaction.id,
                &interaction.token,
                &ir
            ).exec().await.map_err(|e| e.to_string()).map(|_|());
        }
    };
    if let Some(e) = update_race_run(&interaction, |rr| rr.report_user_time(ut), state).await {
        println!("Error updating race run: {}", e);
        let ir = update_resp_to_plain_content(
            "Something went wrong reporting your time. Please ping FoxLisk.");
        return state.interaction_client().create_response(
            interaction.id,
            &interaction.token,
            &ir
        ).exec().await.map_err(|e| e.to_string()).map(|_|());
    }
    let ir = InteractionResponse {
        kind: InteractionResponseType::UpdateMessage,
        data: Some(InteractionResponseData {
            components: Some(vec![Component::ActionRow(
                    ActionRow {
                        components: vec![
                            button_component("VoD ready", CUSTOM_ID_VOD_READY, ButtonStyle::Success)
                        ]
                    }
                )]),
            content: Some("Please click here once your VoD is ready".to_string()),
            ..Default::default()
        })
    };
    state.interaction_client().create_response(interaction.id, &interaction.token, &ir)
        .exec()
        .await
        .map_err(|e| e.to_string())
        .map(|_|())
}



async fn handle_modal_submission(interaction: Box<ModalSubmitInteraction>, state: &Arc<State>) -> Result<(), String> {
    match interaction.data.custom_id.as_str() {
        CUSTOM_ID_USER_TIME_MODAL => handle_user_time_modal(interaction, state).await,
        _ => {
            println!("Unhandled modal: {:?}", interaction);
            Ok(())
        }
    }

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
        Interaction::MessageComponent(mc) => {
            if let Err(e) = handle_button_interaction(mc, &state).await {
                println!("Error handling button: {}", e);
            }
        },
        Interaction::ModalSubmit(ms) => {
            if let Err(e) = handle_modal_submission(ms, &state).await {
                println!("Error handling modal submission: {}", e);
            }
        }
        _ => {}
    }
}

async fn set_application_commands(gc: &Box<GuildCreate>, state: Arc<State>) -> Result<(), String> {
    let cb = CommandBuilder::new(
        CREATE_RACE_CMD.to_string(),
        "Create an asynchronous race for two players".to_string(),
        CommandType::ChatInput,
    )
    .default_member_permissions(Permissions::MANAGE_GUILD)
    .option(CommandOption::User(BaseCommandOptionData {
        description: "First racer".to_string(),
        description_localizations: None,
        name: "p1".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::User(BaseCommandOptionData {
        description: "Second racer".to_string(),
        description_localizations: None,
        name: "p2".to_string(),
        name_localizations: None,
        required: true,
    }))
    .build();

    let resp = state
        .interaction_client()
        .set_guild_commands(gc.id.clone(), &[cb])
        .exec()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!(
            "Error response setting guild commands: {}",
            resp.status()
        ));
    }

    match resp.model().await {
        Ok(cmds) => {
            println!("Got commands for guild {}:", gc.id);

            for cmd in cmds {
                // TODO: role-based permissions
                println!("{:?}", cmd);
            }
            Ok(())
        }
        Err(e) => {
            println!("Error inspecting list of returned commands: {}", e);
            Ok(())
        }
    }
}

async fn handle_event(event: Event, state: Arc<State>) {
    match event {
        Event::GuildCreate(gc) => {
            if let Err(e) = set_application_commands(&gc, state).await {
                println!(
                    "Error setting application commands for guild {:?}: {}",
                    gc.id, e
                );
            }
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