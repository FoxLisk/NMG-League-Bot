use std::time::Duration;
use crate::utils::env_default;

pub mod async_race_worker;
pub mod racetime_scanner_worker;

fn get_tick_duration(env_var: &str) -> Duration {
    Duration::from_secs(env_default(env_var, 60))
}
