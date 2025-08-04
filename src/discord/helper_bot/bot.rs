use std::{collections::HashMap, sync::Arc};

use bb8::Pool;
use diesel::{associations::HasTable, SqliteConnection};
use log::{info, warn};
use nmg_league_bot::{
    config::CONFIG,
    db::DieselConnectionManager,
    models::{
        guild_race_criteria::{
            race_criteria_by_guild_id, GuildCriteria, GuildRaceCriteria, NewGuildRaceCriteria,
            RestreamStatusCriterion,
        },
        player::Player,
    },
    schema, ApplicationCommandOptionError, NMGLeagueBotError,
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
        command::{
            Command, CommandOption, CommandOptionChoice, CommandOptionChoiceValue,
            CommandOptionType, CommandType,
        },
        interaction::{
            application_command::{CommandData, CommandDataOption},
            Interaction, InteractionData, InteractionType,
        },
    },
    channel::message::{embed::EmbedField, Embed, MessageFlags},
    gateway::payload::incoming::InteractionCreate,
    guild::{
        scheduled_event::{GuildScheduledEvent, PrivacyLevel},
        Permissions,
    },
    http::interaction::{InteractionResponse, InteractionResponseData},
    id::{
        marker::{GuildMarker, ScheduledEventMarker},
        Id,
    },
};
use twilight_util::builder::command::CommandBuilder;

use crate::{
    discord::{
        command_option_default,
        interactions_utils::{
            autocomplete_result, get_subcommand_options, plain_ephemeral_response,
        },
        Webhooks,
    },
    get_focused_opt, get_opt,
    shutdown::Shutdown,
};

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

        if cfg!(feature = "testing") {
            if let Err(e) = bot.interaction_client().set_global_commands(&vec![]).await {
                warn!("Error resetting global commands: {e}");
            }
        } else {
            // this is moderately duplicated with the setting of guild commands below but c'est la vie
            let cmds = application_command_definitions();
            match bot.interaction_client().set_global_commands(&cmds).await {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        warn!("Error setting guild commands: {:?}", resp.text().await)
                    }
                }
                Err(e) => {
                    warn!("Error setting guild commands: {e}");
                }
            }
        }

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

    /// gets a [GuildCriteria] for each guild we're currently in
    ///
    /// the CONFIG.guild_id guild will be first
    pub(super) async fn guild_criteria(&self) -> Result<Vec<GuildCriteria>, NMGLeagueBotError> {
        let guilds_joined = self
            .cache
            .iter()
            .guilds()
            .map(|guild| (guild.key().clone(), guild.value().name().to_string()))
            .collect::<HashMap<_, _>>();
        let mut out = Vec::with_capacity(guilds_joined.len());
        // TODO: probably we should still return the main guild config on error...?
        let mut conn = self.diesel_pool.get().await?;
        let mut criteria = race_criteria_by_guild_id(guilds_joined.keys(), &mut conn)?;

        // just push this to the front; its actual contents don't matter because main discord always
        // wants to sync everything, and is hardcoded that way later
        if let Some(criteria) = criteria.remove(&CONFIG.guild_id) {
            out.push(criteria);
        }
        out.extend(criteria.into_values().collect::<Vec<_>>());
        Ok(out)
    }

    fn interaction_client(&self) -> InteractionClient<'_> {
        self.client.interaction(CONFIG.helper_bot_application_id)
    }

    async fn handle_event(&self, event: Event) {
        match event {
            Event::GuildCreate(gc) => {
                info!("Joined guild {}: {}", gc.id, gc.name);
                let cmds = application_command_definitions();
                // set guild commands in testing only. in real life we want to do global application commands.
                if cfg!(feature = "testing") {
                    match self
                        .interaction_client()
                        .set_guild_commands(gc.id, &cmds)
                        .await
                    {
                        Ok(resp) => {
                            if !resp.status().is_success() {
                                warn!("Error setting guild commands: {:?}", resp.text().await)
                            } else {
                                let m = resp.model().await;
                                info!("guild commands: {m:?}");
                            }
                        }
                        Err(e) => {
                            warn!("Error setting guild commands: {e}");
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
        let interaction_data = std::mem::take(&mut interaction.data);
        let mut conn = match self.diesel_pool.get().await {
            Ok(c) => c,
            Err(e) => {
                warn!("Error: unable to obtain diesel connection: {e}");
                self.webhooks
                    .message_error(&format!(
                        "Error getting diesel connection to respond to interaction: {e}"
                    ))
                    .await
                    .ok();
                // we could try to respond to the user here, but we can't do a very good job and I think it's okay if this
                // kind of instantly-fatal internal error just leads to a visible failure for the user
                return;
            }
        };
        match interaction_data {
            Some(data) => match data {
                InteractionData::ApplicationCommand(ac) => {
                    self.handle_application_interaction(interaction, ac, &mut conn)
                        .await;
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
    async fn handle_application_interaction(
        &self,
        interaction: Interaction,
        ac: Box<CommandData>,
        conn: &mut SqliteConnection,
    ) {
        let token = interaction.token.clone();
        let id = interaction.id;
        let resp = match ac.name.as_str() {
            TEST_CMD => handle_test(ac).await,
            TEST_ERROR_CMD => handle_test_error(ac).await,
            CRITERIA_CMD => handle_criteria_commands(interaction, ac, conn).await,
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
                    .create_response(id, &token, &r)
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
                    id,
                    &token,
                    &plain_ephemeral_response(
                        "Sorry, an error occurred trying to handle that command. Maybe try again?",
                    ),
                ).await.ok();
            }
        };
    }
}

async fn handle_criteria_commands(
    interaction: Interaction,
    mut ac: Box<CommandData>,
    conn: &mut SqliteConnection,
) -> Result<InteractionResponse, NMGLeagueBotError> {
    let guild_id = match interaction.guild_id {
        Some(g) => g,
        None => {
            return Ok(plain_ephemeral_response(
                "This command is only meant to be called inside a Discord server.",
            ));
        }
    };
    if CONFIG.guild_id == guild_id {
        return Ok(match interaction.kind {
            InteractionType::ApplicationCommand => {
                plain_ephemeral_response("This is the main discord and connot be filtered.")
            }
            InteractionType::ApplicationCommandAutocomplete => autocomplete_result(vec![]),
            _ => {
                warn!("Super unhandled interaction in main guild: {interaction:?}");
                plain_ephemeral_response("This is the main discord and cannot be filtered. Also what the fuck is this interaction??")
            }
        });
    }
    let (subcommand, opts) = get_subcommand_options(std::mem::take(&mut ac.options))?;
    match (subcommand.as_str(), interaction.kind) {
        (CRITERIA_ADD_SUBCMD, InteractionType::ApplicationCommand) => {
            handle_add_criteria(guild_id, opts, conn)
        }
        (CRITERIA_REMOVE_SUBCMD, InteractionType::ApplicationCommand) => {
            handle_remove_criteria(guild_id, opts, conn)
        }
        (CRITERIA_SHOW_SUBCMD, InteractionType::ApplicationCommand) => {
            handle_show_criteria(guild_id, conn)
        }
        (CRITERIA_ADD_SUBCMD, InteractionType::ApplicationCommandAutocomplete) => {
            handle_add_criteria_autocomplete(opts, conn)
        }
        (CRITERIA_REMOVE_SUBCMD, InteractionType::ApplicationCommandAutocomplete) => {
            handle_remove_criteria_autocomplete(guild_id, opts, conn)
        }
        (_, InteractionType::ApplicationCommandAutocomplete) => {
            warn!(
                "handle_criteria_command got an unexpected autocomplete interaction: {interaction:?}"
            );
            Ok(autocomplete_result(vec![]))
        }
        (_, InteractionType::ApplicationCommand) => {
            warn!("handle_criteria_command got an unexpected application command: {interaction:?}");
            Ok(plain_ephemeral_response(
                "I'm sorry, i somehow don't recognize that command",
            ))
        }
        (_, _) => {
            warn!("handle_criteria_command got something REALLY unexpected: {interaction:?}");
            // TODO: gross
            Err(NMGLeagueBotError::Other("Unexpected command".to_string()))
        }
    }
}

fn handle_add_criteria_autocomplete(
    mut opts: Vec<CommandDataOption>,
    conn: &mut SqliteConnection,
) -> Result<InteractionResponse, NMGLeagueBotError> {
    use diesel::prelude::*;
    use schema::players;

    let partial_name = get_focused_opt!("player", &mut opts, String)?;
    let players: Vec<(i32, String)> = if partial_name.is_empty() {
        Player::table()
            .select((players::id, players::name))
            .limit(25)
            .load(conn)?
    } else {
        schema::players::table
            .filter(players::dsl::name.like(format!("%{partial_name}%")))
            .select((players::id, players::name))
            .limit(25)
            .load(conn)?
    };

    let opts = players
        .into_iter()
        .map(|(id, name)| CommandOptionChoice {
            name: name,
            name_localizations: None,
            value: CommandOptionChoiceValue::String(id.to_string()),
        })
        .collect::<Vec<_>>();

    Ok(autocomplete_result(opts))
}

fn handle_remove_criteria_autocomplete(
    guild_id: Id<GuildMarker>,
    mut opts: Vec<CommandDataOption>,
    conn: &mut SqliteConnection,
) -> Result<InteractionResponse, NMGLeagueBotError> {
    get_focused_opt!("criteria", &mut opts, String)?;
    let grfs = grfs_with_display(guild_id, conn)?;
    let opts = grfs
        .into_iter()
        .map(|(grf, text)| CommandOptionChoice {
            name: text,
            name_localizations: None,
            value: CommandOptionChoiceValue::String(grf.id.to_string()),
        })
        .collect::<Vec<_>>();

    Ok(autocomplete_result(opts))
}

fn grfs_with_display(
    guild_id: Id<GuildMarker>,
    conn: &mut SqliteConnection,
) -> Result<Vec<(GuildRaceCriteria, String)>, NMGLeagueBotError> {
    let grfs = GuildRaceCriteria::list_for_guild(guild_id, conn)?;
    let all_player_ids = grfs
        .iter()
        .map(|grf| grf.player_id.clone())
        .filter_map(|i| i)
        .collect::<Vec<_>>();
    let players = Player::by_id(Some(all_player_ids), conn)?;

    Ok(grfs
        .into_iter()
        .map(|grf| {
            let text = grf.display(grf.player_id.and_then(|pid| players.get(&pid)));
            (grf, text)
        })
        .collect::<Vec<_>>())
}

fn handle_remove_criteria(
    guild_id: Id<GuildMarker>,
    mut opts: Vec<CommandDataOption>,
    conn: &mut SqliteConnection,
) -> Result<InteractionResponse, NMGLeagueBotError> {
    // this is a required param
    let id = match get_opt!("criteria", &mut opts, String)?.parse::<i32>() {
        Ok(i) => i,
        Err(_e) => {
            return Ok(plain_ephemeral_response(
                "It looks like your autocomplete didn't work right. Please try again.",
            ));
        }
    };
    let filter = GuildRaceCriteria::get_by_id(id, guild_id, conn)?;
    if let Some(f) = filter {
        f.delete(conn)?;
        Ok(plain_ephemeral_response(
            "Criteria deleted! Your events will update appropriately in the next few minutes.",
        ))
    } else {
        Ok(plain_ephemeral_response(
            "No matching criteria found. Please try again.",
        ))
    }
}

fn handle_add_criteria(
    guild_id: Id<GuildMarker>,
    mut opts: Vec<CommandDataOption>,
    conn: &mut SqliteConnection,
) -> Result<InteractionResponse, NMGLeagueBotError> {
    // i dont like this nesting, but the errors are different and idk its fine
    // also see comment on the definition of this command for some type nonsense
    let player = match get_opt!("player", &mut opts, String) {
        Ok(s) => match s.parse::<i32>() {
            Ok(id) => match Player::get_by_id(id, conn)? {
                Some(p) => Some(p),
                None => {
                    return Ok(plain_ephemeral_response(
                        "No player by that name was found. Please try again.",
                    ));
                }
            },
            Err(_) => {
                return Ok(plain_ephemeral_response(
                    "It looks like your autocomplete didn't work right. Please try again.",
                ));
            }
        },

        Err(ApplicationCommandOptionError::MissingOption(_)) => None,
        Err(e) => {
            warn!("Error fetching player: {e}");
            return Ok(plain_ephemeral_response(
                "There was an internal error. Please try again.",
            ));
        }
    };

    let restream = get_opt!("restream", &mut opts, Integer)?;
    let restream_status = match restream {
        RESTREAM_REQUIRED => RestreamStatusCriterion::HasRestream,
        RESTREAM_FORBIDDEN => RestreamStatusCriterion::HasNoRestream,
        RESTREAM_AGNOSTIC => RestreamStatusCriterion::Any,
        _ => {
            return Ok(plain_ephemeral_response(
                "Invalid restream option. Please try again.",
            ));
        }
    };
    NewGuildRaceCriteria::new(guild_id, player, restream_status).save(conn)?;

    Ok(plain_ephemeral_response("Criteria added! You'll see relevant races now. If such races already exist, they will sync in the next few minutes."))
}

fn handle_show_criteria(
    guild_id: Id<GuildMarker>,
    conn: &mut SqliteConnection,
) -> Result<InteractionResponse, NMGLeagueBotError> {
    let grfs = grfs_with_display(guild_id, conn)?;

    let fields = grfs
        .into_iter()
        .map(|(_, text)| EmbedField {
            inline: false,
            name: "".to_string(),
            value: text,
        })
        .collect::<Vec<_>>();
    let title = Some(
        if fields.is_empty() {
            "You have not configured any criteria yet, so no NMG League race events are being synced to this server."
        } else {
            "You will see NMG League races that match any of these criteria:"
        }
        .to_string(),
    );

    Ok(InteractionResponse {
        kind: twilight_model::http::interaction::InteractionResponseType::ChannelMessageWithSource,
        data: Some(InteractionResponseData {
            embeds: Some(vec![Embed {
                fields,
                author: None,
                color: None,
                description: None,
                footer: None,
                image: None,
                kind: "rich".to_string(),
                provider: None,
                thumbnail: None,
                timestamp: None,
                title,
                url: None,
                video: None,
            }]),
            flags: Some(MessageFlags::EPHEMERAL),
            ..Default::default()
        }),
    })
}

async fn handle_test(_ac: Box<CommandData>) -> Result<InteractionResponse, NMGLeagueBotError> {
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
const CRITERIA_CMD: &'static str = "criteria";
const CRITERIA_ADD_SUBCMD: &'static str = "add";
const CRITERIA_REMOVE_SUBCMD: &'static str = "remove";
const CRITERIA_SHOW_SUBCMD: &'static str = "show";

const RESTREAM_REQUIRED: i64 = 1;
const RESTREAM_FORBIDDEN: i64 = 2;
const RESTREAM_AGNOSTIC: i64 = 3;

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

    let criteria_commands =
        CommandBuilder::new(CRITERIA_CMD, "Criteria commands", CommandType::ChatInput)
            .default_member_permissions(Permissions::ADMINISTRATOR)
            .option(CommandOption {
                description: "add new criteria.".to_string(),
                kind: CommandOptionType::SubCommand,
                name: CRITERIA_ADD_SUBCMD.to_string(),
                options: Some(vec![
                    CommandOption {
                        description: "restream status".to_string(),
                        kind: CommandOptionType::Integer,
                        name: "restream".to_string(),
                        choices: Some(vec![
                            CommandOptionChoice {
                                name: "Must have restream".to_string(),
                                name_localizations: None,
                                value: CommandOptionChoiceValue::Integer(RESTREAM_REQUIRED),
                            },
                            CommandOptionChoice {
                                name: "Must *not* have restream".to_string(),
                                name_localizations: None,
                                value: CommandOptionChoiceValue::Integer(RESTREAM_FORBIDDEN),
                            },
                            CommandOptionChoice {
                                name: "Either way is fine".to_string(),
                                name_localizations: None,
                                value: CommandOptionChoiceValue::Integer(RESTREAM_AGNOSTIC),
                            },
                        ]),
                        required: Some(true),
                        ..command_option_default()
                    },
                    CommandOption {
                        description:
                            "player you're interested in (omit if you're interested in everyone)"
                                .to_string(),
                        // this has to be a String for autocomplete reasons - the autocompletion can have
                        // a value different than the text it shows the user, but they have to be the same *type*
                        // so we're passing ids as strings in order to allow us to give readable player names
                        kind: CommandOptionType::String,
                        name: "player".to_string(),
                        required: Some(false),
                        autocomplete: Some(true),
                        ..command_option_default()
                    },
                ]),
                ..command_option_default()
            })
            .option(CommandOption {
                description: "remove criteria".to_string(),
                kind: CommandOptionType::SubCommand,
                name: CRITERIA_REMOVE_SUBCMD.to_string(),
                options: Some(vec![CommandOption {
                    description: "criteria to remove".to_string(),
                    kind: CommandOptionType::String,
                    name: "criteria".to_string(),
                    required: Some(true),
                    autocomplete: Some(true),
                    ..command_option_default()
                }]),
                ..command_option_default()
            })
            .option(CommandOption {
                description: "show current criteria".to_string(),
                kind: CommandOptionType::SubCommand,
                name: CRITERIA_SHOW_SUBCMD.to_string(),
                ..command_option_default()
            })
            .build();

    let mut cmds = vec![criteria_commands];

    if cfg!(feature = "testing") {
        cmds.extend(vec![test, test_err]);
    }
    cmds
}
