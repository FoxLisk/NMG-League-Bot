use once_cell::sync::Lazy;
use crate::utils::env_var;

const WEBSITE_URL_VAR: &'static str = "WEBSITE_URL";

pub static CONFIG: Lazy<Config> = Lazy::new(|| Config::new_from_env());

pub struct Config {
    pub website_url: String,
}

impl Config {
    fn new_from_env() -> Self {
        Self {
            website_url: env_var(WEBSITE_URL_VAR)
        }
    }
}