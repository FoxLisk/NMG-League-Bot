use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use serenity::builder::CreateApplicationCommands;
use serenity::client::{ClientBuilder, Context, EventHandler};
use serenity::futures::StreamExt;
use serenity::http::Http;
use serenity::json::Value;
use serenity::model::gateway::Ready;
use serenity::model::guild::{Guild, PartialGuild, Role};
use serenity::model::id::{GuildId, RoleId, UserId};
use serenity::model::interactions::application_command::{
    ApplicationCommandInteraction, ApplicationCommandOptionType, ApplicationCommandType,
};
use serenity::model::interactions::message_component::{
    ActionRow, ButtonStyle, InputTextStyle, MessageComponentInteraction,
};
use serenity::model::interactions::modal::ModalSubmitInteraction;
use serenity::model::interactions::{Interaction, InteractionResponseType};
use serenity::model::prelude::application_command::ApplicationCommandInteractionDataOption;
use serenity::model::prelude::message_component::ActionRowComponent;
use serenity::model::user::User;
use serenity::model::Permissions;
use serenity::prelude::{GatewayIntents, TypeMapKey};
use serenity::utils::MessageBuilder;
use serenity::{async_trait, CacheAndHttp, Client};
use sqlx::SqlitePool;
use tokio::sync::RwLock;

use crate::constants::{APPLICATION_ID_VAR, FOXLISK_USER_ID, TOKEN_VAR};
use crate::db::get_pool;
use crate::models::race::{NewRace, Race};
use crate::models::race_run::RaceRun;
use crate::shutdown::Shutdown;
use crate::utils::send_response;

extern crate rand;
extern crate serenity;
extern crate sqlx;
extern crate tokio;

const CUSTOM_ID_START_RUN: &str = "start_run";
const CUSTOM_ID_FINISH_RUN: &str = "finish_run";
const CUSTOM_ID_FORFEIT_RUN: &str = "forfeit_run";
const CUSTOM_ID_VOD_READY: &str = "vod_ready";
const CUSTOM_ID_VOD_MODAL: &str = "vod_modal";
const CUSTOM_ID_VOD: &str = "vod";
const CUSTOM_ID_USER_TIME: &str = "user_time";
const CUSTOM_ID_USER_TIME_MODAL: &str = "user_time_modal";

struct AdminRoleMap;

impl TypeMapKey for AdminRoleMap {
    type Value = Arc<RwLock<HashMap<GuildId, RoleId>>>;
}

struct RaceHandler;

enum Error {
    BadInput(String),
    APIError(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::BadInput(s) => {
                write!(f, "Bad input: {}", s)
            }
            Error::APIError(s) => {
                write!(f, "Internal error: {}", s)
            }
        }
    }
}

impl RaceHandler {
    const TEST_CMD: &'static str = "test";
    const CREATE_RACE_CMD: &'static str = "create_race";

    fn application_commands(
        commands: &mut CreateApplicationCommands,
    ) -> &mut CreateApplicationCommands {
        commands
            .create_application_command(|command| {
                command
                    .name(Self::TEST_CMD)
                    .description("Test that the bot is alive")
                    .kind(ApplicationCommandType::ChatInput)
                    .default_member_permissions(Permissions::ADMINISTRATOR)
            })
            .create_application_command(|command| {
                command
                    .name(Self::CREATE_RACE_CMD)
                    .description("Create an asynchronous race for two players")
                    .kind(ApplicationCommandType::ChatInput)
                    // N.B. this is imperfect; the Serenity library does not yet support
                    // setting role-based permissions on slash commands, so this is a stand-in
                    .default_member_permissions(Permissions::MANAGE_GUILD)
                    .create_option(|opt| {
                        opt.name("p1")
                            .description("First racer")
                            .kind(ApplicationCommandOptionType::User)
                            .required(true)
                    })
                    .create_option(|opt| {
                        opt.name("p2")
                            .description("Second racer")
                            .kind(ApplicationCommandOptionType::User)
                            .required(true)
                    })
            })
    }

    async fn handle_test(
        &self,
        http: impl AsRef<Http>,
        interaction: ApplicationCommandInteraction,
    ) {
        if let Err(e) = interaction
            .create_interaction_response(http, |ir| {
                ir.kind(InteractionResponseType::ChannelMessageWithSource)
                    .interaction_response_data(|data| data.content("Help, I'm alive!"))
            })
            .await
        {
            println!("Error sending test pong, lol: {}", e);
        }
    }

    fn option_to_user_id(opt: ApplicationCommandInteractionDataOption) -> Result<UserId, Error> {
        if ApplicationCommandOptionType::User != opt.kind {
            return Err(Error::APIError("Bad parameter: Expected user".to_string()));
        }

        if let Value::String(s) = opt
            .value
            .ok_or(Error::APIError("Missing user Id".to_string()))?
        {
            Ok(UserId(s.parse::<u64>().map_err(|_e| {
                Error::APIError("Invalid user Id format".to_string())
            })?))
        } else {
            Err(Error::APIError(
                "Bad parameter: Expected valid user id".to_string(),
            ))
        }
    }

    async fn can_create_race(
        &self,
        ctx: &Context,
        guild_id: Option<GuildId>,
        user: User,
    ) -> Result<bool, String> {
        if let Some(gid) = guild_id {
            let needed_role: RoleId = ctx
                .data
                .read()
                .await
                .get::<AdminRoleMap>()
                .unwrap()
                .read()
                .await
                .get(&gid)
                .ok_or(format!("I don't know what role is needed in guild {}", gid))?
                .clone();
            user.has_role(&ctx, gid, needed_role)
                .await
                .map_err(|e| format!("Error checking if user has role: {}", e))
        } else {
            println!("Command called outside of a guild context");
            Ok(false)
        }
    }

    async fn get_racers(
        ctx: &Context,
        mut options: HashMap<String, ApplicationCommandInteractionDataOption>,
    ) -> Result<(User, User), Error> {
        let p1 = options
            .remove("p1")
            .ok_or(Error::APIError("Missing racer 1 parameter".to_string()))?;
        let p2 = options
            .remove("p2")
            .ok_or(Error::APIError("Missing racer 2 parameter".to_string()))?;

        let racer_1 = Self::option_to_user_id(p1)?;
        let racer_2 = Self::option_to_user_id(p2)?;
        if racer_1 == racer_2 {
            return Err(Error::BadInput("Those are the same player!".to_string()));
        }

        let u1 = racer_1
            .to_user(&ctx)
            .await
            .map_err(|e| Error::BadInput(format!("Error looking up user {}: {}", racer_1, e)))?;
        let u2 = racer_2
            .to_user(&ctx)
            .await
            .map_err(|e| Error::BadInput(format!("Error looking up user {}: {}", racer_1, e)))?;

        Ok((u1, u2))
    }

    async fn notify_racer(
        ctx: &Context,
        mut race_run: RaceRun,
        race: &Race,
        pool: &SqlitePool,
    ) -> Result<(), String> {
        // if race_run.racer_id() != FOXLISK_USER_ID {
        //     println!("Not DMing anyone but myself yet!");
        //     return Ok(());
        // }
        let user: User = race_run
            .racer_id()
            .to_user(ctx)
            .await
            .map_err(|e| e.to_string())?;
        match user
            .direct_message(ctx, |m| {
                m.components(|cmp| {
                    cmp.create_action_row(|row| {
                        row.create_button(|btn| {
                            btn.label("Start run")
                                .custom_id(CUSTOM_ID_START_RUN)
                                .style(ButtonStyle::Primary)
                        })
                    })
                });
                m.content(format!(
                    "Hello, your asynchronous race is now ready.
When you're ready to begin your race, click \"Start run\" and you will be given
filenames to enter.

If anything goes wrong, tell an admin there was an issue with race `{}`",
                    race.uuid
                ));
                m
            })
            .await
        {
            Ok(m) => {
                race_run.set_message_id(m.id);
                race_run.save(pool).await
            }
            Err(e) => {
                println!("Error sending dm: {}", e);
                Err(e.to_string())
            }
        }
    }

    async fn handle_create_race(
        &self,
        ctx: &Context,
        mut interaction: ApplicationCommandInteraction,
    ) -> Result<(), String> {
        let merr = match self
            .can_create_race(
                &ctx,
                std::mem::take(&mut interaction.guild_id),
                std::mem::take(&mut interaction.user),
            )
            .await
        {
            Ok(true) => None,
            Ok(false) => Some("You are not authorized to create races"),
            Err(e) => {
                println!("Error checking if user can create a race: {}", e);
                Some("Internal error")
            }
        };
        if let Some(err) = merr {
            return send_response(&ctx.http, interaction, err).await;
        }

        let options = interaction
            .data
            .options
            .drain(0..)
            .map(|o| (o.name.clone(), o))
            .collect::<HashMap<String, ApplicationCommandInteractionDataOption>>();
        let (r1, r2) = match Self::get_racers(&ctx, options).await {
            Ok(rs) => rs,
            Err(Error::APIError(e)) => {
                println!("Error finding out racer info: {}", e);
                return send_response(&ctx.http, interaction, "Internal error finding racers")
                    .await;
            }
            Err(Error::BadInput(e)) => {
                return send_response(&ctx.http, interaction, e).await;
            }
        };

        let race_insert = NewRace::new();
        let runs = {
            let d = ctx.data.read().await;
            let pool = d.get::<Pool>().unwrap();

            match race_insert.save(&pool).await {
                Ok(race) => match race.select_racers(r1.id, r2.id, &pool).await {
                    Ok(r) => Some((race, r)),
                    Err(e) => {
                        println!("Error selecting racers: {}", e);
                        None
                    }
                },
                Err(e) => {
                    println!("Error persisting race: {}", e);
                    None
                }
            }
        };

        if let Some((race, (run_1, run_2))) = runs {
            let (first, second) = {
                let d = ctx.data.read().await;
                let pool = d.get::<Pool>().unwrap();
                tokio::join!(
                    Self::notify_racer(&ctx, run_1, &race, &pool),
                    Self::notify_racer(&ctx, run_2, &race, &pool)
                )
            };
            if let Err(err) = first.and(second) {
                println!("Error creating race: {}", err);
                send_response(&ctx.http, interaction, "Internal error creating race").await
            } else {
                interaction
                    .create_interaction_response(&ctx.http, |ir| {
                        ir.kind(InteractionResponseType::ChannelMessageWithSource)
                            .interaction_response_data(|data| {
                                data.content(
                                    MessageBuilder::new()
                                        .push("Race created for users ")
                                        .mention(&r1)
                                        .push(" and ")
                                        .mention(&r2)
                                        .build(),
                                )
                            })
                    })
                    .await
                    .map_err(|e| {
                        println!("Error creating response: {}", e);
                        e.to_string()
                    })
            }
        } else {
            send_response(&ctx.http, interaction, "Error creating race").await
        }
    }

    async fn handle_run_forfeit(
        ctx: &Context,
        interaction: MessageComponentInteraction,
        mut race_run: RaceRun,
    ) -> Result<(), String> {
        race_run.forfeit();
        {
            let d = ctx.data.read().await;
            let pool = d.get::<Pool>().unwrap();
            if let Err(e) = race_run.save(&pool).await {
                println!("Error saving race: {}", e);
            }
        }
        interaction.create_interaction_response(&ctx.http, |ir|
            ir.kind(InteractionResponseType::UpdateMessage)
                .interaction_response_data(|ird|
                    ird.content("You have forfeited this match. Please let the admins know if there are any issues.")
                        .components(|cmp| cmp)
                )
        ).await.map(|_|()).map_err(|e|e.to_string())
    }

    async fn handle_run_finish(
        ctx: &Context,
        interaction: MessageComponentInteraction,
        mut race_run: RaceRun,
    ) -> Result<(), String> {
        race_run.finish();
        {
            let d = ctx.data.read().await;
            let pool = d.get::<Pool>().unwrap();
            if let Err(e) = race_run.save(&pool).await {
                println!("Error saving race: {}", e);
            }
        }

        interaction
            .create_interaction_response(&ctx.http, |ir| {
                ir.kind(InteractionResponseType::Modal)
                    .interaction_response_data(|ird| {
                        ird.content("Please enter finish time in **H:MM:SS** format")
                            .custom_id(CUSTOM_ID_USER_TIME_MODAL)
                            .title("Enter finish time in **H:MM:SS** format")
                            .components(|cmps| {
                                cmps.create_action_row(|ar| {
                                    ar.create_input_text(|it| {
                                        it.custom_id(CUSTOM_ID_USER_TIME)
                                            .label("Finish time:")
                                            .style(InputTextStyle::Short)
                                    })
                                })
                            })
                    })
            })
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn handle_run_start(
        ctx: &Context,
        interaction: MessageComponentInteraction,
        mut race_run: RaceRun,
    ) -> Result<(), String> {
        race_run.start();
        let res = {
            let d = ctx.data.read().await;
            let pool = d.get::<Pool>().unwrap();
            race_run.save(pool).await
        };
        match res {
            Ok(_) => interaction
                .create_interaction_response(&ctx.http, |ir| {
                    ir.kind(InteractionResponseType::UpdateMessage)
                        .interaction_response_data(|ird| {
                            ird.components(|cmps| {
                                cmps.create_action_row(|r| {
                                    r.create_button(|input| {
                                        input
                                            .label("Finish run")
                                            .custom_id(CUSTOM_ID_FINISH_RUN)
                                            .style(ButtonStyle::Success)
                                    })
                                        .create_button(|btn| {
                                            btn.label("Forfeit")
                                                .custom_id(CUSTOM_ID_FORFEIT_RUN)
                                                .style(ButtonStyle::Danger)
                                        })
                                })
                            }).content(format!(
                                "Good luck! your filenames are: `{}`

If anything goes wrong, tell an admin there was an issue with race `254bb3a6-5d23-4198-80bb-40f9994298c9`
", race_run.filenames().unwrap()
                            ))
                        })
                })
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Err(e) => {
                println!("Error updating race run: {}", e);
                interaction
                    .edit_original_interaction_response(&ctx.http, |res| {
                        res.content("There was an error starting your race. Please ping FoxLisk")
                    })
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        }
    }

    fn get_user_input_from_modal_field(
        mut data: Vec<ActionRow>,
        custom_id: &str,
    ) -> Result<String, String> {
        if let Some(row) = data.pop() {
            for arc in row.components {
                match arc {
                    ActionRowComponent::InputText(it) => {
                        if it.custom_id == custom_id {
                            return Ok(it.value);
                        }
                    }
                    _ => {
                        println!("Unexpected component");
                        return Err("Unexpected component".to_string());
                    }
                }
            }
            Err(format!("Field {} not found in modal", custom_id))
        } else {
            Err("No action row?".to_string())
        }
    }

    async fn _handle_vod_modal(
        ctx: &Context,
        mut interaction: ModalSubmitInteraction,
    ) -> Result<(), String> {
        let mrr: Option<RaceRun> = {
            let d = ctx.data.read().await;
            let pool = d.get::<Pool>().unwrap();
            RaceRun::get_by_message_id(&interaction.message.clone().unwrap().id, &pool).await?
        };

        if let Some(mut rr) = mrr {
            let user_input = Self::get_user_input_from_modal_field(
                std::mem::take(&mut interaction.data.components),
                CUSTOM_ID_VOD,
            )?;
            rr.set_vod(user_input);

            {
                let d = ctx.data.read().await;
                let pool = d.get::<Pool>().unwrap();
                rr.save(&pool).await?;
            }
            interaction.create_interaction_response(&ctx.http, |cir|
                cir.kind(InteractionResponseType::UpdateMessage)
                    .interaction_response_data(|ird|
                        ird.content(
                            "Thank you, your race is completed. Please message the admins if there are any issues."
                        )
                            .components(|cmp|cmp)
                    )
            ).await
                .map(|_|())
                .map_err(|e| e.to_string())
        } else {
            println!("Unknown message id {:?}", interaction.message);
            Err("I don't know what race to set this vod on".to_string())
        }
    }

    async fn _handle_user_time_modal(
        ctx: &Context,
        mut interaction: ModalSubmitInteraction,
    ) -> Result<(), String> {
        let mrr: Option<RaceRun> = {
            let d = ctx.data.read().await;
            let pool = d.get::<Pool>().unwrap();
            RaceRun::get_by_message_id(&interaction.message.clone().unwrap().id, &pool).await?
        };

        if let Some(mut rr) = mrr {
            let user_input = Self::get_user_input_from_modal_field(
                std::mem::take(&mut interaction.data.components),
                CUSTOM_ID_USER_TIME,
            )?;
            rr.report_user_time(user_input);
            {
                let d = ctx.data.read().await;
                let pool = d.get::<Pool>().unwrap();
                rr.save(&pool).await?;
            }
            interaction
                .create_interaction_response(&ctx.http, |cir| {
                    cir.kind(InteractionResponseType::UpdateMessage)
                        .interaction_response_data(|ird| {
                            ird.content("Please click here once your VoD is ready")
                                .components(|cmp| {
                                    cmp.create_action_row(|ar| {
                                        ar.create_button(|btn| {
                                            btn.label("VoD ready")
                                                .custom_id(CUSTOM_ID_VOD_READY)
                                                .style(ButtonStyle::Success)
                                        })
                                    })
                                })
                        })
                })
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        } else {
            println!("Unknown message id {:?}", interaction.message);
            Err("Unknown whatever I'm exhausted".to_string())
        }
    }

    async fn handle_vod_ready(
        ctx: &Context,
        interaction: MessageComponentInteraction,
    ) -> Result<(), String> {
        interaction
            .create_interaction_response(&ctx.http, |ir| {
                ir.kind(InteractionResponseType::Modal)
                    .interaction_response_data(|ird| {
                        ird.title("VoD URL")
                            .custom_id(CUSTOM_ID_VOD_MODAL)
                            .components(|cmps| {
                                cmps.create_action_row(|row| {
                                    row.create_input_text(|it| {
                                        it.label("Enter VoD here")
                                            .custom_id(CUSTOM_ID_VOD)
                                            .style(InputTextStyle::Short)
                                    })
                                })
                            })
                    })
            })
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn handle_race_run_modal(
        &self,
        ctx: &Context,
        interaction: ModalSubmitInteraction,
    ) -> Result<(), String> {
        match interaction.data.custom_id.as_str() {
            CUSTOM_ID_USER_TIME_MODAL => Self::_handle_user_time_modal(ctx, interaction).await,
            CUSTOM_ID_VOD_MODAL => Self::_handle_vod_modal(ctx, interaction).await,
            _ => Err(format!("Unexpected modal: {}", interaction.data.custom_id)),
        }
    }

    async fn handle_race_run_button(
        &self,
        ctx: &Context,
        interaction: MessageComponentInteraction,
    ) -> Result<(), String> {
        let mrr: Option<RaceRun> = {
            let d = ctx.data.read().await;
            let pool = d.get::<Pool>().unwrap();
            RaceRun::get_by_message_id(&interaction.message.id, &pool).await?
        };

        if let Some(rr) = mrr {
            println!("Got interaction for race {} - {:?}", rr.id, interaction);
            match interaction.data.custom_id.as_str() {
                CUSTOM_ID_START_RUN => Self::handle_run_start(ctx, interaction, rr).await,
                CUSTOM_ID_FINISH_RUN => Self::handle_run_finish(ctx, interaction, rr).await,
                CUSTOM_ID_VOD_READY => Self::handle_vod_ready(ctx, interaction).await,
                CUSTOM_ID_FORFEIT_RUN => Self::handle_run_forfeit(ctx, interaction, rr).await,
                _ => {
                    println!("Unhandled interaction");
                    Err("Unhandled interaction".to_string())
                }
            }
        } else {
            // TODO: Look up based on other fields
            Err(format!("Unknown message id: {}", interaction.message.id))
        }
    }
}

async fn maybe_update_admin_role(ctx: &Context, role: Role) -> Option<Role> {
    if role.name.to_lowercase() == "admin" {
        let data = ctx.data.write().await;
        let role_map = data.get::<AdminRoleMap>().unwrap();
        role_map
            .write()
            .await
            .insert(role.guild_id.clone(), role.id.clone());
        Some(role)
    } else {
        None
    }
}

#[async_trait]
impl EventHandler for RaceHandler {
    async fn cache_ready(&self, _ctx: Context, _guilds: Vec<GuildId>) {
        println!("Cache ready");
    }

    async fn guild_create(&self, ctx: Context, guild: Guild, is_new: bool) {
        println!("Guild create for guild {:?} (is new? {}", guild, is_new);

        let set_commands_result = guild
            .set_application_commands(&ctx.http, Self::application_commands)
            .await;
        println!("Set commands: {:?}", set_commands_result);
        for role in guild.roles.into_values() {
            if let Some(_) = maybe_update_admin_role(&ctx, role).await {
                break;
            }
        }
    }

    async fn guild_role_create(&self, ctx: Context, new: Role) {
        println!("Guild role created: {:?}", new);
        maybe_update_admin_role(&ctx, new).await;
    }

    async fn guild_unavailable(&self, _ctx: Context, guild_id: GuildId) {
        println!("guild unavailable: {}", guild_id);
    }

    async fn guild_update(
        &self,
        _ctx: Context,
        _old_data_if_available: Option<Guild>,
        new_but_incomplete: PartialGuild,
    ) {
        println!("Guild update: {:?}", new_but_incomplete);
    }

    async fn ready(&self, _ctx: Context, data_about_bot: Ready) {
        println!("Ready! {:?}", data_about_bot);
        for guild in &data_about_bot.guilds {
            println!("Guild {:?}", guild);
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        println!("Got interaction: {:?}", interaction);
        match interaction {
            Interaction::ApplicationCommand(i) => {
                println!("interaction name: {}", i.data.name);
                match i.data.name.as_str() {
                    Self::TEST_CMD => {
                        self.handle_test(&ctx.http, i).await;
                    }
                    Self::CREATE_RACE_CMD => {
                        if let Err(e) = self.handle_create_race(&ctx, i).await {
                            println!("Error creating race: {}", e);
                        }
                    }
                    _ => {
                        println!("Unknown interaction :( :(");
                    }
                }
            }
            Interaction::MessageComponent(mci) => {
                println!("Message component interaction: {:?}", mci);
                match self.handle_race_run_button(&ctx, mci).await {
                    Ok(_) => {}
                    Err(e) => {
                        println!("Error handling interaction: {}", e);
                    }
                }
            }
            Interaction::ModalSubmit(msi) => {
                println!("Modal submit: {:?}", msi);
                match self.handle_race_run_modal(&ctx, msi).await {
                    Ok(_) => {}
                    Err(e) => {
                        println!("Error handling interaction: {}", e);
                    }
                }
            }
            _ => {
                println!("Unexpected interaction");
            }
        }
    }
}

struct Pool;
impl TypeMapKey for Pool {
    type Value = SqlitePool;
}

pub(crate) async fn launch_discord_bot(
    mut shutdown_recv: tokio::sync::broadcast::Receiver<Shutdown>,
) -> (Arc<CacheAndHttp>, Arc<RwLock<HashMap<GuildId, RoleId>>>) {
    // https://discord.com/api/oauth2/authorize?client_id=982863079555600414&permissions=122675080256&scope=bot%20applications.commands
    let tok = dotenv::var(TOKEN_VAR).expect(&*format!("{} not found in environment", TOKEN_VAR));
    let application_id = dotenv::var(APPLICATION_ID_VAR)
        .expect(&*format!("{} not found in environment", APPLICATION_ID_VAR))
        .parse::<u64>()
        .expect("Application ID was not a valid u64");
    let pool = get_pool().await.expect("Cannot connect to sqlite database");

    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_MESSAGE_REACTIONS
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::DIRECT_MESSAGE_REACTIONS
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILD_MEMBERS
        // adding guild presences *solely* to get guild members populated in the cache to avoid
        // subsequent http requests
        | GatewayIntents::GUILD_PRESENCES;
    let guild_map = Arc::new(RwLock::new(HashMap::<GuildId, RoleId>::new()));
    let mut client: Client = ClientBuilder::new(&tok, intents)
        .application_id(application_id)
        .event_handler(RaceHandler {})
        .type_map_insert::<AdminRoleMap>(guild_map.clone())
        .type_map_insert::<Pool>(pool)
        .await
        .unwrap();

    let shard_manager = client.shard_manager.clone();
    let cache_and_http = client.cache_and_http.clone();

    tokio::spawn(async move {
        shutdown_recv.recv().await.ok();
        shard_manager.lock().await.shutdown_all().await;
        println!("Discord shutting down gracefully");
    });
    tokio::spawn(async move {
        if let Err(e) = client.start().await {
            println!("Error starting bot: {}", e);
        }
    });

    (cache_and_http, guild_map)
}
