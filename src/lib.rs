extern crate core;
extern crate rand;
extern crate serde_json;
extern crate swiss_pairings;

use crate::config::CONFIG;
use bb8::RunError;
use diesel::ConnectionError;
#[cfg(feature = "racetime_bot")]
use racetime::Error;
use racetime_api::err::RacetimeError;
use thiserror::Error;
use twilight_http::response::DeserializeBodyError;
use twilight_model::id::marker::ChannelMarker;
use twilight_model::id::Id;
use twilight_model::util::datetime::TimestampParseError;
use twilight_validate::message::MessageValidationError;
use twilight_validate::request::ValidationError;
use twitch_api::helix::ClientRequestError;

pub mod config;
pub mod constants;
pub mod db;
pub mod models;
pub mod racetime_types;
pub mod schema;
pub mod twitch_client;
pub mod utils;
pub mod worker_funcs;

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

        let commentary_discussion = CONFIG.commentary_discussion_channel_id;

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

// N.B. this should probably live in web::api, but that's currently not included in the lib
// so that's a huge mess
#[derive(Debug, Error)]
pub enum ApiError {
    #[error("Cannot delete qualifiers that are already closed.")]
    CannotDeletePastQualifiers,
}

// See above comment (but replace with racetime_bot)
#[cfg(feature = "racetime_bot")]
#[derive(Debug, Error)]
pub enum RaceTimeBotError {
    #[error("Error interacting with RaceTime: {0}")]
    RaceTimeError(#[from] racetime::Error),
    #[error("Trying to create a race for the wrong category")]
    InvalidCategory,
    #[error("No auth token (this probably should never happen)")]
    NoAuthToken,
    #[error("That BracketRaceInfo was already known about with a different slug: {0}")]
    ConflictingSlug(i32),
    #[error("Missing BracketRaceInfo for slug {0}")]
    MissingBRI(String),
    #[error("Worker thread for this race disconnected")]
    WorkerDisconnect,
    #[error("Handler disconnected (the inverse of WorkerDisconnect)")]
    HandlerDisconnect,
}

#[derive(Error, Debug)]
pub enum NMGLeagueBotError {
    #[error("Twilight HTTP Error: {0}")]
    TwilightHttpError(#[from] twilight_http::Error),

    #[error("Error validating Discord message: {0}")]
    MessageValidationError(#[from] MessageValidationError),

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

    #[error("API Error: {0}")]
    ApiError(#[from] ApiError),

    #[error("No timestamp on new bracket race info")]
    MissingTimestamp,

    #[error("{0}")]
    TimestampParseError(#[from] TimestampParseError),

    #[error("{0}")]
    DeserializeBodyError(#[from] DeserializeBodyError),

    #[error("{0}")]
    ValidationError(#[from] ValidationError),

    #[error("{0}")]
    Bb8Error(#[from] RunError<ConnectionError>),

    #[cfg(feature = "racetime_bot")]
    #[error("{0}")]
    RaceTimeBotError(#[from] RaceTimeBotError),
}

#[cfg(feature = "racetime_bot")]
impl From<RaceTimeBotError> for racetime::Error {
    fn from(value: RaceTimeBotError) -> Self {
        Error::Custom(Box::new(value))
    }
}
