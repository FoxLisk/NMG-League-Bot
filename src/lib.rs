extern crate core;
extern crate rand;
extern crate serde_json;
extern crate swiss_pairings;

use std::str::FromStr;
use twilight_model::id::Id;
use twilight_model::id::marker::ChannelMarker;
use crate::constants::{COMMENTARY_DISCUSSION_CHANNEL_ID_VAR, COMMPORTUNITIES_CHANNEL_ID_VAR, MATCH_RESULTS_CHANNEL_ID_VAR, SIRIUS_INBOX_CHANNEL_ID_VAR, ZSR_CHANNEL_ID_VAR};

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
            Id::from_str(&std::env::var(COMMPORTUNITIES_CHANNEL_ID_VAR).unwrap()).unwrap();
        let sirius_inbox =
            Id::from_str(&std::env::var(SIRIUS_INBOX_CHANNEL_ID_VAR).unwrap()).unwrap();

        let zsr = Id::from_str(&std::env::var(ZSR_CHANNEL_ID_VAR).unwrap()).unwrap();

        let commentary_discussion =
            Id::from_str(&std::env::var(COMMENTARY_DISCUSSION_CHANNEL_ID_VAR).unwrap()).unwrap();

        let match_results =
            Id::from_str(&std::env::var(MATCH_RESULTS_CHANNEL_ID_VAR).unwrap()).unwrap();
        Self {
            commportunities,
            sirius_inbox,
            zsr,
            commentary_discussion,
            match_results,
        }
    }
}
