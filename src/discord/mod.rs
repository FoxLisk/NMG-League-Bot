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

impl EventHandler for RaceHandler {

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
