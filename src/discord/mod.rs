extern crate rand;
extern crate tokio;

use crate::discord::discord_state::DiscordOperations;
use bb8::RunError;
use chrono::{DateTime, TimeZone};
use diesel::{ConnectionError, SqliteConnection};
use log::{info, warn};
use std::env::VarError;
use std::fmt::{Display, Formatter};
use std::ops::DerefMut;
use std::sync::Arc;
use twilight_http::request::channel::reaction::RequestReactionType;
use twilight_mention::timestamp::{Timestamp as MentionTimestamp, TimestampStyle};
use twilight_mention::Mention;
use twilight_model::application::command::{CommandOption, CommandOptionType};
use twilight_model::application::interaction::application_command::{
    CommandDataOption, CommandOptionValue,
};
use twilight_model::channel::message::embed::EmbedField;
use twilight_model::channel::Message;
use twilight_model::id::marker::UserMarker;
use twilight_model::id::Id;
use twilight_util::builder::embed::EmbedFooterBuilder;

use crate::discord::constants::CUSTOM_ID_START_RUN;
use nmg_league_bot::models::asyncs::race::AsyncRace;
use nmg_league_bot::models::asyncs::race_run::AsyncRaceRun;
use nmg_league_bot::models::bracket_race_infos::BracketRaceInfo;
use nmg_league_bot::models::bracket_races::BracketRace;
use nmg_league_bot::models::player::{MentionOptional, Player};
use nmg_league_bot::utils::{race_to_nice_embeds, ResultErrToString};

use nmg_league_bot::config::CONFIG;
use nmg_league_bot::worker_funcs::{
    clear_commportunities_message, clear_tentative_commentary_assignment_message,
};
use nmg_league_bot::{ApplicationCommandOptionError, BracketRaceState, BracketRaceStateError};
use thiserror::Error;
use twilight_model::channel::message::component::{ActionRow, ButtonStyle};
use twilight_model::channel::message::{Component, Embed};
use twilight_model::guild::Permissions;
pub(crate) use webhooks::Webhooks;

use crate::discord::discord_state::DiscordState;

pub(crate) mod bot;
#[cfg(feature = "helper_bot")]
pub mod helper_bot;
mod webhooks;

mod application_command_definitions;
mod components;
pub(crate) mod discord_state;
mod interaction_handlers;
mod interactions_utils;
mod reaction_handlers;

pub mod constants {
    pub const CUSTOM_ID_START_RUN: &str = "start_run";
    pub const CUSTOM_ID_FINISH_RUN: &str = "finish_run";
    pub const CUSTOM_ID_FORFEIT_RUN: &str = "forfeit_run";

    pub const CUSTOM_ID_FORFEIT_MODAL: &str = "forfeit_modal";
    pub const CUSTOM_ID_FORFEIT_MODAL_INPUT: &str = "forfeit_modal_input";

    pub const CUSTOM_ID_VOD_READY: &str = "vod_ready";

    pub const CUSTOM_ID_VOD_MODAL: &str = "vod_modal";
    pub const CUSTOM_ID_VOD_MODAL_INPUT: &str = "vod";

    pub const CUSTOM_ID_USER_TIME: &str = "user_time";
    pub const CUSTOM_ID_USER_TIME_MODAL: &str = "user_time_modal";

    pub const CREATE_ASYNC_CMD: &str = "create_async";
    pub const CANCEL_ASYNC_CMD: &str = "cancel_async";

    pub const CREATE_SEASON_CMD: &str = "create_season";
    pub const SET_SEASON_STATE_CMD: &str = "set_season_state";
    pub const CREATE_BRACKET_CMD: &str = "create_bracket";
    pub const FINISH_BRACKET_CMD: &str = "finish_bracket";

    pub const ADD_PLAYER_TO_BRACKET_CMD: &str = "add_player_to_bracket";

    pub const CREATE_PLAYER_CMD: &str = "create_player";
    pub const SCHEDULE_RACE_CMD: &str = "schedule_race";
    pub const SUBMIT_QUALIFIER_CMD: &str = "submit_qualifier";
    pub const UPDATE_USER_INFO_CMD: &str = "update_user_info";

    pub const USER_PROFILE_CMD: &str = "See user profile";

    pub const CHECK_USER_INFO_CMD: &str = "check_user_info";
    pub const RESCHEDULE_RACE_CMD: &str = "reschedule_race";
    pub const REPORT_RACE_CMD: &str = "report_race";
    pub const UPDATE_FINISHED_RACE_CMD: &str = "update_finished_race";
    pub const GENERATE_PAIRINGS_CMD: &str = "generate_pairings";

    pub const SEE_UNSCHEDULED_RACES_CMD: &str = "unscheduled_races";
    pub const COMMENTATORS_CMD: &str = "commentators";
    pub const SET_RESTREAM_CMD: &str = "set_restream";
}

// the functions in here aren't well organized
// i sort of want it to be stuff that's doing real work, to separate from the sending commands/
// parsing reactions kind of interface

/// DM the player & save the run model if the DM sends successfully
pub(crate) async fn notify_racer(
    race_run: &mut AsyncRaceRun,
    race: &AsyncRace,
    state: &Arc<DiscordState>,
) -> Result<(), String> {
    let uid = race_run.racer_id()?;
    if Some(uid) == state.cache.current_user().map(|cu| cu.id) {
        info!("Not sending messages to myself");
        race_run.contact_succeeded();
        let mut conn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
        race_run.save(&mut conn).await?;
        return Ok(());
    }
    let dm = state.get_private_channel(uid).await?;
    let content = format!(
        "Hello, your asynchronous race is now ready.
When you're ready to begin your race, click \"Start run\" and you will be given \
filenames to enter.

If anything goes wrong, tell an admin there was an issue with race `{}`",
        race.uuid
    );

    let resp = state
        .discord_client
        .create_message(dm)
        .components(&[Component::ActionRow(ActionRow {
            components: vec![interactions_utils::button_component(
                "Start run",
                CUSTOM_ID_START_RUN,
                ButtonStyle::Primary,
            )],
        })])
        .and_then(|cm| cm.content(&content))
        .map_err(|e| e.to_string())?
        .await
        .map_err(|e| e.to_string())?;

    if resp.status().is_success() {
        let msg = resp.model().await.map_err(|e| e.to_string())?;
        race_run.set_message_id(msg.id.get());
        race_run.contact_succeeded();
        let mut conn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
        race_run.save(&mut conn).await
    } else {
        Err(format!("Error sending message: {}", resp.status()))
    }
}

/// Takes a list of [CommandDataOption]s and tries to find the one with the given name and type. Returns
/// the [CommandOptionValue] inside of the option.
///
/// Note that, due to discord/twilight decisions, if this is an autocomplete option, you'll get back
/// a [`CommandDataValue::Focused(String, <type>)`]
pub fn find_opt(
    name: &str,
    opts: &mut Vec<CommandDataOption>,
    kind: CommandOptionType,
) -> Result<Option<CommandOptionValue>, ApplicationCommandOptionError> {
    let mut i = 0;
    while i < opts.len() {
        if opts[i].name == name {
            break;
        }
        i += 1;
    }
    if i >= opts.len() {
        return Ok(None);
    }
    let actual_kind = opts[i].value.kind();
    if actual_kind != kind {
        return Err(ApplicationCommandOptionError::UnexpectedOptionKind(
            kind,
            actual_kind,
        ));
    }

    Ok(Some(opts.swap_remove(i).value))
}

#[macro_export]
/// Takes a [CommandOptionValue] and unwraps it into the resultant type, if possible
/// e.g. cov_to_val!(option_value, User) would return the `Id<UserMarker>`` (or an error)
// CommandDataOption -> Result<T, ApplicationCommandOptionError>
macro_rules! cov_to_val {
    ($opt:expr, $t: ident) => {
        if let twilight_model::application::interaction::application_command::CommandOptionValue::$t(output) = $opt {
            Ok(output)
        } else {
            Err(nmg_league_bot::ApplicationCommandOptionError::UnexpectedOptionKind(
                twilight_model::application::command::CommandOptionType::$t,
                $opt.kind(),
            ))
        }
    }
}

#[macro_export]
/// Takes an option name (e.g. `"hour"``), a list of options (e.g. `&mut ac.data.options`), and a type to unwrap the option into (e.g. `Integer`)
///
/// Tries to find the option of the given name in the provided options and provide the unwrapped value.
///
/// If it's missing, returns Ok(None). If it's present but has the wrong type, returns Err(ApplicationCommandOptionError::UnexpectedOptionKind)
macro_rules! find_opt {
    ($opt_name:expr, $options:expr, $t:ident) => {
        match crate::discord::find_opt(
            $opt_name,
            $options,
            twilight_model::application::command::CommandOptionType::$t,
        ) {
            Ok(Some(opt)) => $crate::cov_to_val!(opt, $t).map(Some),
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    };
}

#[macro_export]
macro_rules! get_opt {
    ($opt_name:expr, $options:expr, $t:ident) => {
        $crate::find_opt!($opt_name, $options, $t).and_then(|maybe_opt| match maybe_opt {
            Some(opt) => Ok(opt),
            None => Err(
                nmg_league_bot::ApplicationCommandOptionError::MissingOption($opt_name.to_string()),
            ),
        })
    };
}

/**
get_opt_s!("name", &mut vec_of_options, OptionType)

this does something like: find the option with name "name" in the vector,
double check that it has CommandOptionType::OptionType, and then rip the outsides off of the
CommandOptionValue::OptionType(actual_value) and give you back just the actual_value

returns Result<T, String> where actual_value: T

(the `_s` is for `_string` b/c the error type is string)
 */
#[macro_export]
macro_rules! get_opt_s {
    ($opt_name:expr, $options:expr, $t:ident) => {{
        match $crate::get_opt!($opt_name, $options, $t) {
            Ok(o) => Ok(o),
            // for some reason .map_err(|_e|, format!(...)) fails to compile here, complaining
            // about needing type annotations ??
            Err(_e) => Err(format!("Invalid option value for {}", $opt_name)),
        }
    }};
}

#[macro_export]
macro_rules! get_focused_opt {
    ($opt_name:expr, $options:expr, $t:ident) => {{
        $crate::discord::find_opt($opt_name, $options, twilight_model::application::command::CommandOptionType::$t)
            .and_then(|maybe_opt| {
                maybe_opt.ok_or(nmg_league_bot::ApplicationCommandOptionError::MissingOption($opt_name.to_string()))
            }).and_then(|opt| {
                if let twilight_model::application::interaction::application_command::CommandOptionValue::Focused(output, twilight_model::application::command::CommandOptionType::$t) = opt {
                    Ok(output)
                } else {
                    Err(nmg_league_bot::ApplicationCommandOptionError::UnexpectedOptionKind(
                        twilight_model::application::command::CommandOptionType::$t, opt.kind()
                    ))
                }
            })
    }};
}

/// an ErrorResponse indicates that, rather than simply responding to the interaction with some
/// kind of response, you want to both respond to that (with a plain error message)
/// *AND* inform the admins that there was an error
#[derive(Debug)]
pub struct ErrorResponse {
    user_facing_error: String,
    internal_error: String,
}

// TODO: some kind of impl From<&str> for ErrorResponse that returns an error response with "Internal error, sorry."
//       as the user facing error and the &str as the internal one?
//       or maybe From<E: Error> that behaves similarly

impl ErrorResponse {
    fn new<S1: Into<String>, S2: Display>(user_facing_error: S1, internal_error: S2) -> Self {
        Self {
            user_facing_error: user_facing_error.into(),
            internal_error: internal_error.to_string(),
        }
    }

    fn new_internal(internal_error: impl Display) -> Self {
        Self {
            user_facing_error: "Internal error, sorry.".to_string(),
            internal_error: internal_error.to_string(),
        }
    }
}

impl Display for ErrorResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.user_facing_error)?;
        write!(f, " (Internal error: {})", &self.internal_error)
    }
}

#[derive(Error, Debug)]
enum ScheduleRaceError {
    #[error("Connection error: {0}")]
    ConnectionError(#[from] RunError<ConnectionError>),
    #[error("Race already finished")]
    RaceFinished,
    #[error("Database error: {0}")]
    DatabaseError(#[from] diesel::result::Error),
    #[error("Bracket race state error: {0}")]
    BracketRaceStateError(#[from] BracketRaceStateError),
}

impl From<serde_json::Error> for ScheduleRaceError {
    fn from(value: serde_json::Error) -> Self {
        Self::BracketRaceStateError(BracketRaceStateError::ParseError(value))
    }
}

/// Returns a nicely formatted message to return to chat
/// wipes out existing state about scheduling/commentating/etc
async fn schedule_race<Tz: TimeZone>(
    mut the_race: BracketRace,
    when: DateTime<Tz>,
    state: &Arc<DiscordState>,
) -> Result<String, ScheduleRaceError> {
    if the_race.state()? == BracketRaceState::Finished {
        return Err(ScheduleRaceError::RaceFinished);
    }
    let mut cxn = state.diesel_cxn().await?;
    let conn = cxn.deref_mut();

    // TODO: jesus this is gross
    let (old_info, mut new_info) = the_race.schedule(&when, conn)?;

    let p1r = Player::get_by_id(the_race.player_1_id, conn);
    let p1_name = p1r
        .mention_maybe()
        .unwrap_or("Error finding player".to_string());

    let p2r = Player::get_by_id(the_race.player_2_id, conn);
    let p2_name = p2r
        .mention_maybe()
        .unwrap_or("Error finding player".to_string());
    if let (Ok(Some(_p1)), Ok(Some(_p2))) = (p1r, p2r) {
        // clear some old stuff up
        // TODO: parallelize? this method on its own is gonna come close to hitting discord API
        //       limits, so maybe don't bother lol

        if let Err(e) = clear_commportunities_message(
            &mut new_info,
            &state.discord_client,
            &state.channel_config,
        )
        .await
        {
            warn!("Error clearing old commportunities message upon rescheduling: {e}");
        }

        if let Err(e) = clear_tentative_commentary_assignment_message(
            &mut new_info,
            &state.discord_client,
            &state.channel_config,
        )
        .await
        {
            warn!(
                "Error clearing old tentative commentary assignment message upon rescheduling: {e}"
            );
        }

        // TODO: clear ZSR message and commentary assignment message, as well
        //       but those might benefit from messaging
        //       maybe just update messages to say (x vs x, but now rescheduled)?
        //       deal with it later.

        match create_commportunities_post(&new_info, state).await {
            Ok(m) => {
                new_info.set_commportunities_message_id(m.id);
            }
            Err(e) => {
                warn!("Error creating commportunities post: {:?}", e);
            }
        };

        if let Err(e) = new_info.update(conn) {
            warn!("Error updating bracket race info: {:?}", e);
        }
    }

    let new_t = MentionTimestamp::new(when.timestamp() as u64, Some(TimestampStyle::LongDateTime))
        .mention();
    let old_t = old_info
        .scheduled()
        .map(|t| {
            format!(
                " (was {})",
                MentionTimestamp::new(t.timestamp() as u64, Some(TimestampStyle::LongDateTime))
                    .mention()
            )
        })
        .unwrap_or("".to_string());
    Ok(format!(
        "{p1_name} vs {p2_name} has been scheduled for {new_t}{old_t}"
    ))
}

/// "commentary opportunities"
async fn create_commportunities_post(
    info: &BracketRaceInfo,
    state: &Arc<DiscordState>,
) -> Result<Message, String> {
    // TODO: i'm losing my mind at the number of extra SQL queries here
    // it doesn't REALLY matter but alksdjflkajsklfj 😱
    let mut cxn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    let fields = race_to_nice_embeds(info, cxn.deref_mut()).map_err(|e| e.to_string())?;
    let embeds = vec![Embed {
        author: None,
        color: Some(0x00b0f0),
        description: None,
        fields,
        footer: Some(EmbedFooterBuilder::new("React to volunteer").build()),
        image: None,
        kind: "rich".to_string(),
        provider: None,
        thumbnail: None,
        timestamp: None,
        title: Some(format!("New match available for commentary")),
        url: None,
        video: None,
    }];
    let msg = state
        .discord_client
        .create_message(state.channel_config.commportunities.clone())
        .embeds(&embeds)
        .map_err_to_string()?
        .await
        .map_err_to_string()?;

    let m = msg.model().await.map_err_to_string()?;
    let emojum = RequestReactionType::Unicode { name: "🎙" };
    if let Err(e) = state
        .discord_client
        .create_reaction(m.channel_id, m.id, &emojum)
        .await
    {
        warn!(
            "Error adding initial reaction to commportunity post: {:?}",
            e
        );
    }
    Ok(m)
}

pub async fn comm_ids_and_names<D: DiscordOperations>(
    info: &BracketRaceInfo,
    state: &Arc<D>,
    conn: &mut SqliteConnection,
) -> Result<Vec<(Id<UserMarker>, String)>, diesel::result::Error> {
    let comms = info.commentator_signups(conn)?;
    let mut out = vec![];
    for comm in comms {
        if let Ok(id) = comm.discord_id() {
            out.push((id.clone(), state.best_name_for(id.clone()).await));
        }
    }
    Ok(out)
}

fn embed_with_title(fields: Vec<EmbedField>, title: impl Into<String>) -> Embed {
    Embed {
        author: None,
        color: None,
        description: None,
        fields,
        footer: None,
        image: None,
        kind: "rich".to_string(),
        provider: None,
        thumbnail: None,
        timestamp: None,
        title: Some(title.into()),
        url: None,
        video: None,
    }
}

pub fn generate_invite_link() -> Result<String, VarError> {
    let client_id = &CONFIG.discord_client_id;
    let permissions = Permissions::MANAGE_ROLES
        | Permissions::MANAGE_CHANNELS
        | Permissions::MANAGE_NICKNAMES
        | Permissions::CHANGE_NICKNAME
        | Permissions::MANAGE_GUILD_EXPRESSIONS
        | Permissions::MANAGE_WEBHOOKS
        | Permissions::READ_MESSAGE_HISTORY
        | Permissions::MANAGE_EVENTS
        | Permissions::MODERATE_MEMBERS
        | Permissions::SEND_MESSAGES
        | Permissions::SEND_MESSAGES_IN_THREADS
        | Permissions::CREATE_PUBLIC_THREADS
        | Permissions::CREATE_PRIVATE_THREADS
        | Permissions::SEND_TTS_MESSAGES
        | Permissions::MANAGE_MESSAGES
        | Permissions::MANAGE_THREADS
        | Permissions::EMBED_LINKS
        | Permissions::ATTACH_FILES
        | Permissions::MENTION_EVERYONE
        | Permissions::ADD_REACTIONS
        | Permissions::USE_EXTERNAL_EMOJIS
        | Permissions::USE_EXTERNAL_STICKERS
        | Permissions::USE_SLASH_COMMANDS;
    let permissions = permissions.bits() | (1 << 44); // this is CREATE_EVENTS, an undocumented(?) new(?) permission
    Ok(format!("https://discord.com/oauth2/authorize?client_id={client_id}&permissions={permissions}&scope=bot%20applications.commands"))
}

fn command_option_default() -> CommandOption {
    CommandOption {
        autocomplete: None,
        channel_types: None,
        choices: None,
        description: "".to_string(),
        description_localizations: None,
        kind: CommandOptionType::SubCommand,
        max_length: None,
        max_value: None,
        min_length: None,
        min_value: None,
        name: "".to_string(),
        name_localizations: None,
        options: None,
        required: None,
    }
}
