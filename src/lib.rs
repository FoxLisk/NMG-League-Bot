extern crate core;
extern crate rand;
extern crate serde_json;
extern crate swiss_pairings;

use racetime_api::err::RacetimeError;
use thiserror::Error;
use twilight_model::id::marker::ChannelMarker;
use twilight_model::id::Id;
use twitch_api::helix::ClientRequestError;
use crate::config::CONFIG;

pub mod constants;
pub mod db;
pub mod models;
pub mod racetime_types;
pub mod schema;
pub mod twitch_client;
pub mod utils;
pub mod worker_funcs;
pub mod config;

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
        let commportunities = CONFIG.commportunities_channel_id;
        let sirius_inbox = CONFIG.sirius_inbox_channel_id;

        let zsr = CONFIG.zsr_channel_id;

        let commentary_discussion =
            CONFIG.commentary_discussion_channel_id;

        let match_results = CONFIG.match_results_channel_id;
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
    TwilightHttpError(#[from] twilight_http::Error),

    #[error("Database error: {0}")]
    DatabaseError(#[from] diesel::result::Error),

    #[error("[De]serialization error: {0}")]
    SerdeError(#[from] serde_json::Error),

    #[error("Illegal state transition: {0:?}")]
    StateError(String),

    #[error("RaceTime error: {0}")]
    RaceTimeError(#[from] RacetimeError),

    #[error("Twitch API error: {0}")]
    TwitchError(#[from] ClientRequestError<reqwest::Error>),
}
