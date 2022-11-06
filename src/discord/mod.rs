extern crate rand;
extern crate tokio;

use std::fmt::{Display, Formatter};
use std::ops::DerefMut;
use std::sync::Arc;

use bb8::RunError;
use chrono::{DateTime, Duration, TimeZone};
use diesel::ConnectionError;
use twilight_http::error::ErrorType;
use twilight_http::request::channel::reaction::RequestReactionType;
use twilight_http::Client;
use twilight_mention::timestamp::{Timestamp as MentionTimestamp, TimestampStyle};
use twilight_mention::Mention;
use twilight_model::application::command::CommandOptionType;
use twilight_model::application::component::button::ButtonStyle;
use twilight_model::application::component::{ActionRow, Component};
use twilight_model::application::interaction::application_command::CommandDataOption;
use twilight_model::channel::embed::Embed;
use twilight_model::channel::Message;
use twilight_model::id::marker::GuildMarker;
use twilight_model::id::Id;
use twilight_model::scheduled_event::{GuildScheduledEvent, PrivacyLevel};
use twilight_model::util::Timestamp as ModelTimestamp;
use twilight_util::builder::embed::EmbedFooterBuilder;

use crate::discord::constants::CUSTOM_ID_START_RUN;
use nmg_league_bot::models::bracket_race_infos::BracketRaceInfo;
use nmg_league_bot::models::bracket_races::{BracketRace, BracketRaceState, BracketRaceStateError};
use nmg_league_bot::models::player::{MentionOptional, Player};
use nmg_league_bot::models::race::Race;
use nmg_league_bot::models::race_run::RaceRun;
use nmg_league_bot::utils::{race_to_nice_embeds, ResultErrToString};
use nmg_league_bot::{ChannelConfig, NMGLeagueBotError};
use thiserror::Error;
use nmg_league_bot::worker_funcs::{clear_commportunities_message, clear_tentative_commentary_assignment_message};
pub(crate) use webhooks::Webhooks;

use crate::discord::discord_state::DiscordState;

pub(crate) mod bot;
mod webhooks;

mod application_commands;
mod components;
pub(crate) mod discord_state;
mod interaction_handlers;
mod interactions_utils;
mod reaction_handlers;

mod constants {
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

    pub const CREATE_RACE_CMD: &str = "create_race";
    pub const CANCEL_RACE_CMD: &str = "cancel_race";
    pub const ADMIN_ROLE_NAME: &'static str = "Admin";

    pub const CREATE_SEASON_CMD: &str = "create_season";
    pub const CREATE_BRACKET_CMD: &str = "create_bracket";

    pub const ADD_PLAYER_TO_BRACKET_CMD: &str = "add_player_to_bracket";
    pub const CREATE_PLAYER_CMD: &str = "create_player";
    pub const SCHEDULE_RACE_CMD: &str = "schedule_race";
    pub const RESCHEDULE_RACE_CMD: &str = "reschedule_race";
    pub const REPORT_RACE_CMD: &str = "report_race";
    pub const UPDATE_FINISHED_RACE_CMD: &str = "update_finished_race";
    pub const GENERATE_PAIRINGS_CMD: &str = "generate_pairings";
}

// the functions in here aren't well organized
// i sort of want it to be stuff that's doing real work, to separate from the sending commands/
// parsing reactions kind of interface

/// DM the player & save the run model if the DM sends successfully
pub(crate) async fn notify_racer(
    race_run: &mut RaceRun,
    race: &Race,
    state: &Arc<DiscordState>,
) -> Result<(), String> {
    let uid = race_run.racer_id()?;
    if Some(uid) == state.cache.current_user().map(|cu| cu.id) {
        println!("Not sending messages to myself");
        race_run.contact_succeeded();
        let mut conn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
        race_run.save(&mut conn).await?;
        return Ok(());
    }
    let dm = state.get_private_channel(uid).await?;
    let content = format!(
        "Hello, your asynchronous race is now ready.
When you're ready to begin your race, click \"Start run\" and you will be given
filenames to enter.

If anything goes wrong, tell an admin there was an issue with race `{}`",
        race.uuid
    );

    let resp = state
        .client
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
        .exec()
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

/// extracts the option with given name, if any
/// does not preserve order of remaining opts
/// returns a string representing the error if the expected opt is not found
pub fn get_opt(
    name: &str,
    opts: &mut Vec<CommandDataOption>,
    kind: CommandOptionType,
) -> Result<CommandDataOption, String> {
    let mut i = 0;
    while i < opts.len() {
        if opts[i].name == name {
            break;
        }
        i += 1;
    }
    if i >= opts.len() {
        return Err(format!("Unable to find expected option {}", name));
    }
    let actual_kind = opts[i].value.kind();
    if actual_kind != kind {
        return Err(format!(
            "Option {} was of unexpected type (got {}, expected {})",
            name,
            actual_kind.kind(),
            kind.kind()
        ));
    }

    Ok(opts.swap_remove(i))
}

/**
get_opt!("name", &mut vec_of_options, OptionType)

this does something like: find the option with name "name" in the vector,
double check that it has CommandOptionType::OptionType, and then rip the outsides off of the
CommandOptionValue::OptionType(actual_value) and give you back just the actual_value

returns Result<T, String> where actual_value: T
 */
#[macro_export]
macro_rules! get_opt {
    ($opt_name:expr, $options:expr, $t:ident) => {{
        crate::discord::get_opt($opt_name, $options, twilight_model::application::command::CommandOptionType::$t).and_then(|opt| {
            if let twilight_model::application::interaction::application_command::CommandOptionValue::$t(output) = opt.value {
                Ok(output)
            } else {
                Err(format!("Invalid option value for {}", $opt_name))
            }
        })
    }};
}

#[macro_export]
macro_rules! get_focused_opt {
    ($opt_name:expr, $options:expr, $t:ident) => {{
        crate::discord::get_opt($opt_name, $options, twilight_model::application::command::CommandOptionType::$t).and_then(|opt| {
            if let twilight_model::application::interaction::application_command::CommandOptionValue::Focused(output, twilight_model::application::command::CommandOptionType::$t) = opt.value {
                Ok(output)
            } else {
                Err(format!("Invalid option value for {}", $opt_name))
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

impl ErrorResponse {
    fn new<S1: Into<String>, S2: Display>(user_facing_error: S1, internal_error: S2) -> Self {
        Self {
            user_facing_error: user_facing_error.into(),
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
    if let (Ok(Some(p1)), Ok(Some(p2))) = (p1r, p2r) {
        let bracket_name = the_race
            .bracket(conn)
            .map(|b| b.name)
            .unwrap_or("".to_string());
        match create_or_update_event(
            bracket_name,
            &old_info,
            &new_info,
            &p1,
            &p2,
            state.guild_id(),
            &state.client,
        )
        .await
        {
            Ok(evt) => {
                new_info.set_scheduled_event_id(evt.id);
            }
            Err(e) => {
                println!("Error creating event: {}", e);
            }
        };

        // clear some old stuff up
        // TODO: parallelize? this method on its own is gonna come close to hitting discord API
        // limits, so maybe don't bother lol

        if let Err(e) = clear_commportunities_message(&mut new_info, &state.client, &state.channel_config).await {
            println!("Error clearing old commportunities message upon rescheduling: {e}");
        }

        if let Err(e) = clear_tentative_commentary_assignment_message(&mut new_info, &state.client, &state.channel_config).await {
            println!(
                "Error clearing old tentative commentary assignment message upon rescheduling: {e}"
            );
        }

        // TODO: clear ZSR message and commentary assignment message, as well
        // but those might benefit from messaging
        // maybe just update messages to say (x vs x, but now rescheduled)?
        // deal with it later.

        match create_commportunities_post(&new_info, state).await {
            Ok(m) => {
                new_info.set_commportunities_message_id(m.id);
            }
            Err(e) => {
                println!("Error creating commportunities post: {:?}", e);
            }
        };

        if let Err(e) = new_info.update(conn) {
            println!("Error updating bracket race info: {:?}", e);
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
        .client
        .create_message(state.channel_config.commportunities.clone())
        .embeds(&embeds)
        .map_err_to_string()?
        .exec()
        .await
        .map_err_to_string()?;

    let m = msg.model().await.map_err_to_string()?;
    let emojum = RequestReactionType::Unicode { name: "🎙" };
    if let Err(e) = state
        .client
        .create_reaction(m.channel_id, m.id, &emojum)
        .exec()
        .await
    {
        println!(
            "Error adding initial reaction to commportunity post: {:?}",
            e
        );
    }
    Ok(m)
}

async fn create_or_update_event(
    bracket_name: String,
    old_info: &BracketRaceInfo,
    new_info: &BracketRaceInfo,
    p1: &Player,
    p2: &Player,
    gid: Id<GuildMarker>,
    client: &Client,
) -> Result<GuildScheduledEvent, String> {
    let when = new_info
        .scheduled()
        .ok_or("Error: No timestamp on new bracket race info".to_string())?;
    let start = ModelTimestamp::from_secs(when.timestamp()).map_err(|e| e.to_string())?;
    let end = ModelTimestamp::from_secs((when.clone() + Duration::minutes(100)).timestamp())
        .map_err(|e| e.to_string())?;

    let req = if let Some(existing_id) = old_info.get_scheduled_event_id() {
        client
            .update_guild_scheduled_event(gid, existing_id)
            .scheduled_start_time(&start)
            .scheduled_end_time(Some(&end))
            .exec()
    } else {
        client
            .create_guild_scheduled_event(gid, PrivacyLevel::GuildOnly)
            .external(
                &format!("{bracket_name}: {} vs {}", p1.name, p2.name),
                &format!(
                    "https://multistre.am/{}/{}/layout4/",
                    p1.twitch_user_login, p2.twitch_user_login
                ),
                &start,
                &end,
            )
            .map_err(|e| e.to_string())?
            .exec()
    };
    let resp = req.await.map_err(|e| e.to_string())?;
    resp.model().await.map_err(|e| {
        format!(
            "Error setting event id on race: couldn't deserialize body?! {}",
            e
        )
    })
}
