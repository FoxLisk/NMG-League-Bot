extern crate core;
extern crate rand;
extern crate serde_json;
extern crate swiss_pairings;

use std::str::FromStr;
use twilight_model::id::Id;
use twilight_model::id::marker::ChannelMarker;
use crate::constants::{COMMENTARY_DISCUSSION_CHANNEL_ID_VAR, COMMPORTUNITIES_CHANNEL_ID_VAR, MATCH_RESULTS_CHANNEL_ID_VAR, SIRIUS_INBOX_CHANNEL_ID_VAR, ZSR_CHANNEL_ID_VAR};
use crate::utils::env_var;
use thiserror::Error;

pub mod constants;
pub mod db;
pub mod models;
pub mod schema;
pub mod utils;
pub mod worker_funcs;
pub mod racetime_types;


pub struct ChannelConfig {
    pub commportunities: Id<ChannelMarker>,
    pub sirius_inbox: Id<ChannelMarker>,
    pub zsr: Id<ChannelMarker>,
    pub commentary_discussion: Id<ChannelMarker>,
    pub match_results: Id<ChannelMarker>,
}

impl ChannelConfig {
    /// explodes if any env vars are missing
    pub fn new_from_env() -> Self {
        let commportunities =
            Id::from_str(&env_var(COMMPORTUNITIES_CHANNEL_ID_VAR)).unwrap();
        let sirius_inbox =
            Id::from_str(&env_var(SIRIUS_INBOX_CHANNEL_ID_VAR)).unwrap();

        let zsr = Id::from_str(&env_var(ZSR_CHANNEL_ID_VAR)).unwrap();

        let commentary_discussion =
            Id::from_str(&env_var(COMMENTARY_DISCUSSION_CHANNEL_ID_VAR)).unwrap();

        let match_results =
            Id::from_str(&env_var(MATCH_RESULTS_CHANNEL_ID_VAR)).unwrap();
        Self {
            commportunities,
            sirius_inbox,
            zsr,
            commentary_discussion,
            match_results,
        }
    }
}

#[derive(Error, Debug)]
pub enum NMGLeagueBotError {
    #[error("Twilight HTTP Error: {0}")]
    TwilightHttpError(#[from] twilight_http::Error)
}