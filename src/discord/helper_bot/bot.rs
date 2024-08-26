use std::{collections::HashMap, sync::Arc};

use bb8::Pool;
use log::{info, warn};
use nmg_league_bot::{
    config::CONFIG, db::DieselConnectionManager, models::bracket_races::BracketRace,
    NMGLeagueBotError,
};
use tokio::sync::broadcast::Receiver;
use tokio_stream::StreamExt;
use twilight_cache_inmemory::InMemoryCache;
use twilight_gateway::{
    stream::{self, ShardEventStream},
    Config, Event, Intents,
};
use twilight_http::{
    client::InteractionClient,
    request::scheduled_event::{CreateGuildScheduledEvent, UpdateGuildScheduledEvent},
    Client,
};
use twilight_model::{
    application::{
        command::{Command, CommandOption, CommandOptionType, CommandType},
        interaction::{application_command::CommandData, Interaction},
    },
    gateway::payload::incoming::InteractionCreate,
    guild::{
        scheduled_event::{GuildScheduledEvent, PrivacyLevel},
        Permissions,
    },
    http::interaction::InteractionResponse,
    id::{
        marker::{GuildMarker, ScheduledEventMarker},
        Id,
    },
};
use twilight_util::builder::command::CommandBuilder;

use crate::{
    discord::{command_option_default, interactions_utils::plain_ephemeral_response, Webhooks},
    get_opt, get_opt_s,
    shutdown::Shutdown,
};

pub(super) struct GuildEventConfig {
    pub(super) guild_id: Id<GuildMarker>,
    #[allow(unused)]
    // very useful in debugging but not necessarily logged outside of development, so allow unused
    pub(super) guild_name: String,
}

impl GuildEventConfig {
    /// is this guild interested in tracking this race?
    pub(super) fn should_sync_race(&self, _race: &BracketRace) -> bool {
        true
    }
}

pub(super) struct HelperBot {
    cache: InMemoryCache,
    client: Client,
    diesel_pool: Pool<DieselConnectionManager>,
    webhooks: Webhooks,
}

impl HelperBot {
    pub(super) fn new(webhooks: Webhooks, diesel_pool: Pool<DieselConnectionManager>) -> Self {
        let cache = InMemoryCache::new();
        let client = Client::new(CONFIG.helper_bot_discord_token.clone());
        Self {
            cache,
            client,
            diesel_pool,
            webhooks,
        }
    }

    pub(super) async fn get_guild_scheduled_events(
        &self,
        guild_id: Id<GuildMarker>,
    ) -> Result<Vec<GuildScheduledEvent>, NMGLeagueBotError> {
        let req = self.client.guild_scheduled_events(guild_id);
        let resp = req.await?;

        let data = resp.models().await?;
        Ok(data)
    }

    pub(super) fn update_scheduled_event(
        &self,
        guild_id: Id<GuildMarker>,
        event_id: Id<ScheduledEventMarker>,
    ) -> UpdateGuildScheduledEvent<'_> {
        self.client.update_guild_scheduled_event(guild_id, event_id)
    }

    pub(super) fn create_scheduled_event(
        &self,
        guild_id: Id<GuildMarker>,
    ) -> CreateGuildScheduledEvent<'_> {
        self.client
            .create_guild_scheduled_event(guild_id, PrivacyLevel::GuildOnly)
    }

    pub(super) async fn run(bot: Arc<Self>, mut shutdown: Receiver<Shutdown>) {
        // i *think* all I care about out are what guilds I'm in
        let intents = Intents::GUILDS;

        let cfg = Config::builder(CONFIG.helper_bot_discord_token.clone(), intents).build();

        let mut shards = stream::create_recommended(&bot.client, cfg, |_, builder| builder.build())
            .await
            // TODO: surface this unwrap? idk
            .unwrap()
            .collect::<Vec<_>>();

        // N.B. collecting these into a vec and then using `.iter_mut()` is stupid, but idk how to
        // convince the compiler that i have an iterator of mutable references in a simpler way
        let mut events = ShardEventStream::new(shards.iter_mut());

        loop {
            tokio::select! {
                Some((_shard_id, evt)) = events.next() => {
                    match evt {
                        Ok(event) => {
                            bot.cache.update(&event);
                            bot.handle_event(event).await;
                        }
                        Err(e) => {
                            warn!("Got error receiving discord event: {e}");
                            if e.is_fatal() {
                                info!("Helper bot shutting down due to fatal error");
                                break;
                            }
                        }
                    }
                },

                _sd = shutdown.recv() => {
                    info!("Helper bot shutting down...");
                    break;
                }
            }
        }
        info!("Helper bot done");
    }

    /// gets a [GuildEventConfig] for each guild we're currently in
    ///
    /// the CONFIG.guild_id guild will be first
    pub(super) fn guild_event_configs(&self) -> Vec<GuildEventConfig> {
        let mut guild_ids = self
            .cache
            .iter()
            .guilds()
            .map(|guild| (guild.key().clone(), guild.value().name().to_string()))
            .collect::<HashMap<_, _>>();
        let mut out = Vec::with_capacity(guild_ids.len());

        if let Some((guild_id, guild_name)) = guild_ids.remove_entry(&CONFIG.guild_id) {
            out.push(GuildEventConfig {
                guild_id,
                guild_name,
            });
        }
        out.extend(
            guild_ids
                .into_iter()
                .map(|(guild_id, guild_name)| GuildEventConfig {
                    guild_id,
                    guild_name,
                }),
        );
        out
    }

    fn interaction_client(&self) -> InteractionClient<'_> {
        self.client.interaction(CONFIG.helper_bot_application_id)
    }

    async fn handle_event(&self, event: Event) {
        match event {
            Event::GuildCreate(gc) => {
                info!("Joined guild {}: {}", gc.id, gc.name);
                let cmds = application_command_definitions();
                // set guild commands in the configured guild, in testing only. in real life we want to do global application commands.
                if cfg!(feature = "testing") && gc.id == CONFIG.guild_id {
                    match self
                        .interaction_client()
                        .set_guild_commands(gc.id, &cmds)
                        .await
                    {
                        Ok(resp) => {
                            if !resp.status().is_success() {
                                warn!(
                                    "Error setting guild commands in main discord: {:?}",
                                    resp.text().await
                                )
                            }
                        }
                        Err(e) => {
                            warn!("Error setting guild commands in main discord: {e}");
                        }
                    }
                }
            }
            Event::InteractionCreate(ic) => {
                self.handle_interaction(ic).await;
            }
            _ => {}
        }
    }

    async fn handle_interaction(&self, ic: Box<InteractionCreate>) {
        let mut interaction = ic.0;
        let id = std::mem::take(&mut interaction.data);
        match id {
            Some(data) => match data {
                twilight_model::application::interaction::InteractionData::ApplicationCommand(
                    ac,
                ) => {
                    self.handle_application_interaction(interaction, ac).await;
                }

                _ => {
                    info!("Got unhandled interaction: {interaction:?}");
                }
            },
            None => {
                warn!("Got Interaction with no data, ignoring {interaction:?}");
            }
        }
    }

    // N.B. interaction has had `.data` stripped off of it; thats passed in as `ac` instead
    async fn handle_application_interaction(&self, interaction: Interaction, ac: Box<CommandData>) {
        let resp = match ac.name.as_str() {
            TEST_CMD => handle_test(ac).await,
            TEST_ERROR_CMD => handle_test_error(ac).await,
            _ => {
                warn!("Unhandled application command: {}", ac.name);
                // maybe give a boilerplate response? prolly not
                return;
            }
        };

        match resp {
            Ok(r) => {
                if let Err(e) = self
                    .interaction_client()
                    .create_response(interaction.id, &interaction.token, &r)
                    .await
                {
                    warn!("Error responding to application command: {e}");
                    self.webhooks
                        .message_error(&format!("Error responding to application command: {e}"))
                        .await
                        .ok();
                }
            }
            Err(e) => {
                warn!("Error handling application command: {e}");
                self.webhooks
                    .message_error(&format!("Error handling application command: {e}"))
                    .await
                    .ok();
                self.interaction_client().create_response(
                    interaction.id,
                    &interaction.token,
                    &plain_ephemeral_response(
                        "Sorry, an error occurred trying to handle that command. Maybe try again?",
                    ),
                ).await.ok();
            }
        };
    }
}

async fn handle_test(ac: Box<CommandData>) -> Result<InteractionResponse, NMGLeagueBotError> {
    Ok(plain_ephemeral_response("Hi mom!"))
}

async fn handle_test_error(
    mut ac: Box<CommandData>,
) -> Result<InteractionResponse, NMGLeagueBotError> {
    let err = get_opt!("err", &mut ac.options, String)?;
    Err(NMGLeagueBotError::Other(err))
}

const TEST_CMD: &'static str = "test";
const TEST_ERROR_CMD: &'static str = "test_error";

fn application_command_definitions() -> Vec<Command> {
    let test = CommandBuilder::new(
        TEST_CMD.to_string(),
        "Test command".to_string(),
        CommandType::ChatInput,
    )
    .default_member_permissions(Permissions::ADMINISTRATOR)
    .build();
    let test_err = CommandBuilder::new(
        TEST_ERROR_CMD.to_string(),
        "Produce an error".to_string(),
        CommandType::ChatInput,
    )
    .option(CommandOption {
        description: "The error to produce".to_string(),
        description_localizations: None,
        name: "err".to_string(),
        name_localizations: None,
        required: Some(true),
        kind: CommandOptionType::String,
        ..command_option_default()
    })
    .default_member_permissions(Permissions::ADMINISTRATOR)
    .build();

    let mut cmds = vec![];

    if cfg!(feature = "testing") {
        cmds.extend(vec![test, test_err]);
    }
    cmds
}
