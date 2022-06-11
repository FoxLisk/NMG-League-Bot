pub(crate) mod bot_twilight;
mod webhooks;

pub(crate) use webhooks::Webhooks;

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

use crate::constants::{APPLICATION_ID_VAR, TOKEN_VAR};
use crate::db::get_pool;
use crate::models::race::{NewRace, Race};
use crate::models::race_run::RaceRun;
use crate::shutdown::Shutdown;

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

const CREATE_RACE_CMD: &str = "create_race";
const ADMIN_ROLE_NAME: &'static str = "Admin";

struct AdminRoleMap;

impl TypeMapKey for AdminRoleMap {
    type Value = Arc<RwLock<HashMap<GuildId, RoleId>>>;
}

struct RaceHandler;

impl RaceHandler {
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
        println!("Serenity bot: handle run sundowning");
        return Ok(());
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
    if role.name.to_lowercase() == ADMIN_ROLE_NAME {
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
        println!("Serenity guild create sundowning...");
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
                    _ => {
                        println!("Unhandled ApplicationCommand interaction :( :(");
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
