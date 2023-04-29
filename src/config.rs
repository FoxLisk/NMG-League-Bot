use crate::utils::env_var;
use once_cell::sync::Lazy;
use std::str::FromStr;
use twilight_model::id::marker::{ApplicationMarker, ChannelMarker, GuildMarker};
use twilight_model::id::Id;
use twitch_api::twitch_oauth2::{ClientId, ClientSecret};

const TOKEN_VAR: &str = "DISCORD_TOKEN";
const APPLICATION_ID_VAR: &str = "APPLICATION_ID";
const TWITCH_CLIENT_ID_VAR: &str = "TWITCH_CLIENT_ID";
const TWITCH_CLIENT_SECRET_VAR: &str = "TWITCH_CLIENT_SECRET";

const ASYNC_WEBHOOK_VAR: &str = "ASYNC_WEBHOOK_URL";
const ADMIN_WEBHOOK_VAR: &str = "ADMIN_WEBHOOK_URL";

const COMMPORTUNITIES_CHANNEL_ID_VAR: &str = "COMMPORTUNITIES_CHANNEL_ID";
const SIRIUS_INBOX_CHANNEL_ID_VAR: &str = "SIRIUS_INBOX_CHANNEL_ID";
const ZSR_CHANNEL_ID_VAR: &str = "ZSR_CHANNEL_ID";
const COMMENTARY_DISCUSSION_CHANNEL_ID_VAR: &str = "COMMENTARY_DISCUSSION_CHANNEL_ID";
const MATCH_RESULTS_CHANNEL_ID_VAR: &str = "MATCH_RESULTS_CHANNEL_ID";

const CLIENT_ID_VAR: &str = "CLIENT_ID";
const CLIENT_SECRET_VAR: &str = "CLIENT_SECRET";
const AUTHORIZE_URL_VAR: &str = "AUTHORIZE_URL";
const CANCEL_RACE_TIMEOUT_VAR: &str = "CANCEL_RACE_TIMEOUT";

const CRON_TICKS_VAR: &str = "CRON_TICK_SECS";
const RACETIME_TICK_SECS: &str = "RACETIME_TICK_SECS";

const GUILD_ID_VAR: &str = "LEAGUE_GUILD_ID";

pub const LOG4RS_CONF_FILE_VAR: &str = "LOG4RS_CONFIG_FILE";

const WEBSITE_URL_VAR: &'static str = "WEBSITE_URL";

pub static CONFIG: Lazy<Config> = Lazy::new(|| Config::new_from_env());

pub struct Config {
    pub discord_token: String,
    pub discord_client_id: String,
    pub discord_client_secret: String,
    pub discord_application_id: Id<ApplicationMarker>,

    pub async_webhook: String,
    pub admin_webhook: String,

    pub commportunities_channel_id: Id<ChannelMarker>,
    pub sirius_inbox_channel_id: Id<ChannelMarker>,
    pub zsr_channel_id: Id<ChannelMarker>,
    pub commentary_discussion_channel_id: Id<ChannelMarker>,
    pub match_results_channel_id: Id<ChannelMarker>,

    pub discord_authorize_url: String,

    pub twitch_client_id: ClientId,
    pub twitch_client_secret: ClientSecret,

    pub cancel_race_timeout: u64,
    pub cron_ticks: u64,
    pub racetime_tick_secs: u64,

    pub guild_id: Id<GuildMarker>,

    pub website_url: String,
}

fn id_from_env<T>(k: &str) -> Id<T> {
    Id::<T>::new(parse::<u64>(k))
}

fn parse<T: FromStr>(k: &str) -> T {
    match env_var(k).parse::<T>() {
        Ok(t) => t,
        Err(_e) => {
            panic!(
                "Failed to parse value of {k} as {}",
                std::any::type_name::<T>()
            )
        }
    }
}

impl Config {
    fn new_from_env() -> Self {
        Self {
            discord_token: env_var(TOKEN_VAR),
            discord_application_id: id_from_env(APPLICATION_ID_VAR),
            twitch_client_id: ClientId::new(env_var(TWITCH_CLIENT_ID_VAR)),
            twitch_client_secret: ClientSecret::new(env_var(TWITCH_CLIENT_SECRET_VAR)),
            async_webhook: env_var(ASYNC_WEBHOOK_VAR),
            admin_webhook: env_var(ADMIN_WEBHOOK_VAR),
            commportunities_channel_id: id_from_env(COMMPORTUNITIES_CHANNEL_ID_VAR),
            sirius_inbox_channel_id: id_from_env(SIRIUS_INBOX_CHANNEL_ID_VAR),
            zsr_channel_id: id_from_env(ZSR_CHANNEL_ID_VAR),
            commentary_discussion_channel_id: id_from_env(COMMENTARY_DISCUSSION_CHANNEL_ID_VAR),
            match_results_channel_id: id_from_env(MATCH_RESULTS_CHANNEL_ID_VAR),
            discord_client_id: env_var(CLIENT_ID_VAR),
            discord_client_secret: env_var(CLIENT_SECRET_VAR),
            discord_authorize_url: env_var(AUTHORIZE_URL_VAR),
            cancel_race_timeout: parse(CANCEL_RACE_TIMEOUT_VAR),
            cron_ticks: parse(CRON_TICKS_VAR),
            racetime_tick_secs: parse(RACETIME_TICK_SECS),
            guild_id: id_from_env(GUILD_ID_VAR),
            website_url: env_var(WEBSITE_URL_VAR),
        }
    }
}
