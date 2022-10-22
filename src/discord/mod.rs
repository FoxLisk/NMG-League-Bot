pub(crate) mod bot;
mod webhooks;

use std::fmt::{Display, Formatter};
use crate::discord::discord_state::DiscordState;
use nmg_league_bot::models::race::Race;
use nmg_league_bot::models::race_run::RaceRun;
use std::sync::Arc;
use twilight_model::application::command::CommandOptionType;
use twilight_model::application::component::button::ButtonStyle;
use twilight_model::application::component::{ActionRow, Component};
use twilight_model::application::interaction::application_command::CommandDataOption;
pub(crate) use webhooks::Webhooks;

pub(crate) mod discord_state;
mod interactions_utils;
mod application_commands;
mod interaction_handlers;
mod components;
mod reaction_handlers;

extern crate rand;
extern crate tokio;

const CUSTOM_ID_START_RUN: &str = "start_run";
const CUSTOM_ID_FINISH_RUN: &str = "finish_run";
const CUSTOM_ID_FORFEIT_RUN: &str = "forfeit_run";

const CUSTOM_ID_FORFEIT_MODAL: &str = "forfeit_modal";
const CUSTOM_ID_FORFEIT_MODAL_INPUT: &str = "forfeit_modal_input";

const CUSTOM_ID_VOD_READY: &str = "vod_ready";

const CUSTOM_ID_VOD_MODAL: &str = "vod_modal";
const CUSTOM_ID_VOD_MODAL_INPUT: &str = "vod";

const CUSTOM_ID_USER_TIME: &str = "user_time";
const CUSTOM_ID_USER_TIME_MODAL: &str = "user_time_modal";

const CREATE_RACE_CMD: &str = "create_race";
const CANCEL_RACE_CMD: &str = "cancel_race";
const ADMIN_ROLE_NAME: &'static str = "Admin";

const CREATE_SEASON_CMD: &str = "create_season";
const CREATE_BRACKET_CMD: &str = "create_bracket";

const ADD_PLAYER_TO_BRACKET_CMD: &str = "add_player_to_bracket";
const CREATE_PLAYER_CMD: &str = "create_player";
const SCHEDULE_RACE_CMD: &str = "schedule_race";

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
