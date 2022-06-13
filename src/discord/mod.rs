pub(crate) mod bot;
mod webhooks;
pub(crate) use webhooks::Webhooks;
pub(crate) mod discord_state;

extern crate rand;
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

