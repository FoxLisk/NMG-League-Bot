extern crate core;
extern crate rand;
extern crate serde_json;
extern crate swiss_pairings;

use std::num::ParseIntError;

use crate::config::CONFIG;
use bb8::RunError;
use diesel::ConnectionError;
#[cfg(feature = "racetime_bot")]
use racetime::Error;
use racetime_api::err::RacetimeError;
use thiserror::Error;
use twilight_http::response::DeserializeBodyError;
use twilight_model::application::command::CommandOptionType;
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
#[cfg(test)]
pub mod test_utils;
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
    RaceControllerDisconnect,
    #[error("Handler disconnected (the inverse of WorkerDisconnect)")]
    HandlerDisconnect,
}

#[derive(Debug, thiserror::Error)]
pub enum RaceEventError {
    #[error("Missing player with id {0}")]
    MissingPlayer(i32),
}

#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq, Debug)]
pub enum BracketRaceState {
    New,
    Scheduled,
    Finished,
}

#[derive(Debug, Error)]
pub enum BracketRaceStateError {
    #[error("Invalid state: expected {0:?}, got {1:?}")]
    InvalidState(Vec<BracketRaceState>, BracketRaceState),
    #[error("Deserialization error: {0}")]
    ParseError(#[from] serde_json::Error),
    #[error("Cannot finish race without both players' results")]
    MissingResult,
    #[error("Database error: {0}")]
    DatabaseError(#[from] diesel::result::Error),
}

#[derive(Error, Debug)]
pub enum ApplicationCommandOptionError {
    #[error("Missing option {0}")]
    MissingOption(String),

    #[error("Unexpected option kind: expected {0:?}, got {1:?}")]
    UnexpectedOptionKind(CommandOptionType, CommandOptionType),

    #[error("No subcommand found")]
    NoSubcommand,
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

    #[error("{0}")]
    ParseIntError(#[from] ParseIntError),

    #[cfg(feature = "racetime_bot")]
    #[error("{0}")]
    RaceTimeBotError(#[from] RaceTimeBotError),

    #[error("Error managing race event: {0}")]
    RaceEventError(#[from] RaceEventError),

    #[error("Error with a BracketRaceState: {0}")]
    RaceStateError(#[from] BracketRaceStateError),

    #[error("Error getting ApplicationCommand options: {0}")]
    ApplicationCommandOptionError(#[from] ApplicationCommandOptionError),

    #[error("Unable to parse finish time")]
    ParseFinishTimeError,

    #[error("Other error: {0}")]
    Other(String),
}

#[cfg(feature = "racetime_bot")]
impl From<RaceTimeBotError> for racetime::Error {
    fn from(value: RaceTimeBotError) -> Self {
        Error::Custom(Box::new(value))
    }
}
