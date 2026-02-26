use crate::discord::components::action_row;
use crate::discord::constants::{
    ADD_PLAYER_TO_BRACKET_CMD, CANCEL_ASYNC_CMD, CHECK_USER_INFO_CMD, COMMENTATORS_CMD,
    CREATE_ASYNC_CMD, CREATE_BRACKET_CMD, CREATE_PLAYER_CMD, CREATE_SEASON_CMD, FINISH_BRACKET_CMD,
    GENERATE_PAIRINGS_CMD, REPORT_RACE_CMD, RESCHEDULE_RACE_CMD, SCHEDULE_RACE_CMD,
    SEE_UNSCHEDULED_RACES_CMD, SET_RESTREAM_CMD, SET_SEASON_STATE_CMD, SUBMIT_QUALIFIER_CMD,
    UPDATE_FINISHED_RACE_CMD, UPDATE_USER_INFO_CMD, USER_PROFILE_CMD,
};

use crate::discord::discord_state::DiscordOperations;
use crate::discord::discord_state::DiscordState;
use crate::discord::interactions_utils::{
    autocomplete_result, button_component, get_subcommand_options, interaction_to_custom_id,
    plain_ephemeral_response, plain_interaction_response, update_resp_to_plain_content,
};
use crate::discord::{self, notify_racer, ErrorResponse, ScheduleRaceError};
use crate::{find_opt, get_focused_opt, get_opt_s};
use nmg_league_bot::models::asyncs::race::{AsyncRace, NewAsyncRace, RaceState};
use nmg_league_bot::models::asyncs::race_run::AsyncRaceRun;
use once_cell::sync::Lazy;
use reqwest::Url;
use std::future::Future;

use chrono::{DateTime, TimeDelta, TimeZone, Utc};

use diesel::result::Error;
use diesel::SqliteConnection;
use either::Either;
use log::{info, warn};
use nmg_league_bot::config::CONFIG;
use nmg_league_bot::models::bracket_races::BracketRace;
use nmg_league_bot::models::brackets::{Bracket, BracketType, NewBracket};
use nmg_league_bot::models::player::{NewPlayer, Player};
use nmg_league_bot::models::player_bracket_entries::NewPlayerBracketEntry;
use nmg_league_bot::models::qualifer_submission::NewQualifierSubmission;
use nmg_league_bot::models::season::{NewSeason, Season, SeasonState};
use nmg_league_bot::utils::{parse_race_result, ResultCollapse, ResultErrToString};
use nmg_league_bot::worker_funcs::{trigger_race_finish, RaceFinishError, RaceFinishOptions};
use nmg_league_bot::{utils, BracketRaceState, BracketRaceStateError, NMGLeagueBotError};
use racetime_api::endpoint::Query;
use racetime_api::endpoints::UserSearch;
use racetime_api::types::UserSearchResult;
use regex::{Regex, RegexBuilder};
use std::ops::DerefMut;
use std::sync::Arc;
use twilight_http::request::application::interaction::UpdateResponse;
use twilight_http::request::channel::message::UpdateMessage;
use twilight_mention::Mention;
use twilight_model::application::command::{CommandOptionChoice, CommandOptionChoiceValue};
use twilight_model::application::interaction::application_command::{
    CommandData, CommandDataOption,
};
use twilight_model::application::interaction::{Interaction, InteractionType};
use twilight_model::channel::message::component::ButtonStyle;
use twilight_model::channel::message::embed::EmbedField;
use twilight_model::channel::message::{Embed, MessageFlags};
use twilight_model::gateway::payload::incoming::InteractionCreate;
use twilight_model::http::interaction::{
    InteractionResponse, InteractionResponseData, InteractionResponseType,
};
use twilight_model::id::marker::{ChannelMarker, MessageMarker, UserMarker};
use twilight_model::id::Id;
use twilight_model::user::User;
use twitch_api::helix::users::GetUsersRequest;

const REALLY_CANCEL_ID: &'static str = "really_cancel";

/// turns a "String" error response into a plain interaction response with that text
///
/// designed for use on admin-only commands, where errors should just be reported to the admins
fn admin_command_wrapper(
    result: Result<Option<InteractionResponse>, String>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    let out = Ok(result
        .map_err(|e| Some(plain_interaction_response(e)))
        .collapse());
    out
}

/// Parameters to pass to an UpdateResponse
// currently this is only content but i wanted an abstraction layer for it. well, I wanted the
// abstraction layer cause I thought I would need components already. but I decided I didn't.
// Oh well.
#[derive(Debug, Default)]
struct UpdateResponseBag {
    content: Option<String>,
}

impl UpdateResponseBag {
    fn new_content<S: Into<String>>(content: S) -> Self {
        Self {
            content: Some(content.into()),
            ..Default::default()
        }
    }

    fn hydrate<'a>(&'a self, mut ur: UpdateResponse<'a>) -> UpdateResponse<'a> {
        if let Some(s) = self.content.as_ref() {
            ur = ur.content(Some(s));
        }
        ur
    }
}

/// Discord requires you to respond to interactions within 3 seconds. This function is to handle the
/// "Send a DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE response, then do the slow work, then update the
/// response" workflow.
fn long_command_wrapper<F, Fut>(
    func: F,
    ac: Box<CommandData>,
    ic: Box<InteractionCreate>,
    state: Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse>
where
    F: FnOnce(Box<CommandData>, Box<InteractionCreate>, Arc<DiscordState>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<UpdateResponseBag, ErrorResponse>> + Send + 'static,
{
    // first we spawn off the long-running job
    tokio::spawn(async move {
        // it's really gross that i can't seem to design this stuff to avoid copying interaction tokens
        let token = ic.token.clone();
        let client = state.interaction_client();
        let r = func(ac, ic, state.clone()).await;
        match r {
            Ok(urb) => {
                let ur = client.update_response(&token);
                if let Err(e) = urb.hydrate(ur).await {
                    state
                        .submit_error(format!(
                            "Error in long_command_wrapper trying to hydrate UpdateResponse: {e}"
                        ))
                        .await;
                }
            }
            Err(ErrorResponse {
                user_facing_error,
                internal_error,
            }) => {
                client
                    .update_response(&token)
                    .content(Some(&user_facing_error))
                    .await
                    .map_err_to_string()
                    .err();
                state.submit_error(internal_error).await;
            }
        }
    });
    // then we immediately return so discord knows we're thinking about it
    Ok(Some(InteractionResponse {
        kind: InteractionResponseType::DeferredChannelMessageWithSource,
        data: None,
    }))
}

/// N.B. interaction.data is already ripped out, here, and is passed in as the first parameter
pub async fn handle_application_interaction(
    ac: Box<CommandData>,
    interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    // general (non-admin) commands
    match ac.name.as_str() {
        SCHEDULE_RACE_CMD => {
            return match interaction.kind {
                InteractionType::ApplicationCommand => {
                    long_command_wrapper(_handle_schedule_race_cmd, ac, interaction, state.clone())
                }
                InteractionType::ApplicationCommandAutocomplete => {
                    handle_schedule_race_autocomplete(ac, interaction, &state)
                        .map(|i| Some(i))
                        .map_err(|e| ErrorResponse::new("Unexpected error thinking about days.", e))
                }
                _ => Err(ErrorResponse::new(
                    "Weird internal error, sorry",
                    format!("Unexpected InteractionType for {}", SCHEDULE_RACE_CMD),
                )),
            };
        }
        SUBMIT_QUALIFIER_CMD => {
            return handle_submit_qualifier(ac, interaction, state).await;
        }
        UPDATE_USER_INFO_CMD => {
            return handle_update_user_info(ac, interaction, state).await;
        }
        CHECK_USER_INFO_CMD => {
            return handle_check_user_info(ac, interaction, state).await;
        }
        USER_PROFILE_CMD => {
            return handle_user_profile(ac, interaction, state).await;
        }

        _ => {}
    };
    match state.application_command_run_by_admin(&interaction).await {
        Ok(true) => {}
        Ok(false) => {
            return Ok(Some(plain_interaction_response(
                "This command is admin-only.",
            )));
        }
        Err(s) => {
            return Err(ErrorResponse::new("Error running command", s));
        }
    };

    // admin commands
    match ac.name.as_str() {
        CREATE_PLAYER_CMD => admin_command_wrapper(
            handle_create_player(ac, interaction, state)
                .await
                .map(|i| Some(i)),
        ),
        ADD_PLAYER_TO_BRACKET_CMD => admin_command_wrapper(
            handle_add_player_to_bracket(ac, interaction, state)
                .await
                .map(|i| Some(i)),
        ),
        // admin commands
        CREATE_ASYNC_CMD => {
            admin_command_wrapper(handle_create_race(ac, state).await.map(Option::from))
        }
        GENERATE_PAIRINGS_CMD => {
            admin_command_wrapper(handle_generate_pairings(ac, state).await.map(Option::from))
        }
        RESCHEDULE_RACE_CMD => {
            match interaction.kind {
                InteractionType::ApplicationCommand => {
                    long_command_wrapper(handle_reschedule_race_cmd, ac, interaction, state.clone())
                }
                InteractionType::ApplicationCommandAutocomplete => {
                    // N.B. this will have to change if/when i add race id autocompletion
                    admin_command_wrapper(
                        handle_schedule_race_autocomplete(ac, interaction, state).map(|i| Some(i)),
                    )
                }
                _ => Ok(Some(plain_interaction_response(format!(
                    "Unexpected InteractionType for {}",
                    RESCHEDULE_RACE_CMD
                )))),
            }
        }

        CANCEL_ASYNC_CMD => admin_command_wrapper(handle_cancel_race(ac, interaction, state).await),
        CREATE_SEASON_CMD => {
            admin_command_wrapper(handle_create_season(ac, state).await.map(Option::from))
        }
        SET_SEASON_STATE_CMD => {
            admin_command_wrapper(handle_set_season_state(ac, state).await.map(Option::from))
        }

        CREATE_BRACKET_CMD => {
            admin_command_wrapper(handle_create_bracket(ac, state).await.map(Option::from))
        }
        FINISH_BRACKET_CMD => {
            admin_command_wrapper(handle_finish_bracket(ac, state).await.map(Option::from))
        }
        REPORT_RACE_CMD => {
            admin_command_wrapper(handle_report_race(ac, state).await.map(Option::from))
        }

        UPDATE_FINISHED_RACE_CMD => {
            admin_command_wrapper(handle_rereport_race(ac, state).await.map(Option::from))
        }

        SEE_UNSCHEDULED_RACES_CMD => admin_command_wrapper(
            handle_see_unscheduled_races(ac, state)
                .await
                .map(Option::from),
        ),

        COMMENTATORS_CMD => admin_command_wrapper(match interaction.kind {
            InteractionType::ApplicationCommand => {
                handle_commentator_command(ac, interaction, state)
                    .await
                    .map(Some)
            }
            InteractionType::ApplicationCommandAutocomplete => {
                scheduled_races_autocomplete(ac, interaction, state)
                    .await
                    .map(Some)
            }
            _ => Ok(Some(plain_interaction_response(format!(
                "Unexpected InteractionType for {}",
                RESCHEDULE_RACE_CMD
            )))),
        }),

        SET_RESTREAM_CMD => admin_command_wrapper(match interaction.kind {
            InteractionType::ApplicationCommand => {
                handle_set_restream(ac, interaction, state).await.map(Some)
            }
            InteractionType::ApplicationCommandAutocomplete => {
                scheduled_races_autocomplete(ac, interaction, state)
                    .await
                    .map(Some)
            }
            _ => Ok(Some(plain_interaction_response(format!(
                "Unexpected InteractionType for {}",
                RESCHEDULE_RACE_CMD
            )))),
        }),

        _ => {
            info!("Unhandled application command: {}", ac.name);
            Ok(None)
        }
    }
}

/// parses a string in `YYYY/MM/DD` format, returns (month, day)
/// if it doesn't parse, does not return that
fn parse_day(s: &str) -> Option<(i32, u32, u32)> {
    // TODO: move regex construction elsewhere, remove unwrap (or take it out of normal code path)
    //       in particular this should be a OnceCell
    let re = Regex::new(r#"(\d{4})/(\d{1,2})/(\d{1,2})"#).unwrap();
    let stripped = s.trim();
    let caps = re.captures(stripped)?;

    let y = caps.get(1)?.as_str().parse().ok()?;
    let m = caps.get(2)?.as_str().parse().ok()?;
    let d = caps.get(3)?.as_str().parse().ok()?;
    Some((y, m, d))
}

fn datetime_from_options(
    day_string: &str,
    hour: i64,
    minute: i64,
    ampm: &str,
) -> Result<DateTime<chrono_tz::Tz>, &'static str> {
    let (y, m, d) = parse_day(&day_string).ok_or(
        "Invalid day. Expected YYYY/MM/DD format. You should see helpful autocomplete options.",
    )?;
    let hour_adjusted = match ampm {
        "AM" => {
            if hour == 12 {
                0
            } else {
                hour
            }
        }
        "PM" => {
            if hour == 12 {
                hour
            } else {
                hour + 12
            }
        }
        _ => {
            return Err("Invalid value for AM/PM");
        }
    };

    chrono_tz::US::Eastern
        .with_ymd_and_hms(y, m, d, hour_adjusted as u32, minute as u32, 0)
        .earliest()
        .ok_or("Invalid datetime")
}

fn get_datetime_from_scheduling_cmd(
    options: &mut Vec<CommandDataOption>,
) -> Result<DateTime<chrono_tz::Tz>, &'static str> {
    let day_string = match get_opt_s!("day", options, String) {
        Ok(d) => d,
        Err(_e) => {
            return Err("Missing required day option");
        }
    };
    let hour = match get_opt_s!("hour", options, Integer) {
        Ok(h) => h,
        Err(_e) => {
            return Err("Missing required hour option");
        }
    };
    let minute = match get_opt_s!("minute", options, Integer) {
        Ok(m) => m,
        Err(_e) => {
            return Err("Missing required minute option");
        }
    };
    // TODO: this cannot possibly be the best way to do this, lmao
    let ampm_offset = match get_opt_s!("am_pm", options, String) {
        Ok(ap) => ap,
        Err(_e) => {
            return Err("Missing required AM/PM option");
        }
    };

    datetime_from_options(&day_string, hour, minute, &ampm_offset)
}

async fn _handle_schedule_race_cmd(
    mut ac: Box<CommandData>,
    mut interaction: Box<InteractionCreate>,
    state: Arc<DiscordState>,
) -> Result<UpdateResponseBag, ErrorResponse> {
    const BLAND_USER_FACING_ERROR: &str = "Internal error. Sorry.";
    let member = std::mem::take(&mut interaction.member).ok_or(ErrorResponse::new(
        BLAND_USER_FACING_ERROR,
        "No member found on schedule_race command!",
    ))?;
    let user = member.user.ok_or(ErrorResponse::new(
        BLAND_USER_FACING_ERROR,
        "No user found on member struct for a schedule_race command!",
    ))?;

    let dt: DateTime<_> = match get_datetime_from_scheduling_cmd(&mut ac.options) {
        Ok(dt) => dt,
        Err(e) => {
            return Ok(UpdateResponseBag::new_content(e.to_string()));
        }
    };

    let mut cxn = state
        .diesel_cxn()
        .await
        .map_err(|e| ErrorResponse::new(BLAND_USER_FACING_ERROR, e))?;
    let player =
        match Player::get_by_discord_id(&user.id.to_string(), cxn.deref_mut()).map_err(|e| {
            ErrorResponse::new(
                BLAND_USER_FACING_ERROR,
                format!("Error fetching player: {e}"),
            )
        })? {
            Some(p) => p,
            None => {
                return Ok(UpdateResponseBag::new_content(
                    "You do not seem to be registered.",
                ));
            }
        };

    let opponent = find_opt!("opponent", &mut ac.options, User).map_err(|e| {
        ErrorResponse::new(
            BLAND_USER_FACING_ERROR,
            format!("Invalid `opponent` parameter in schedule_race command: {e}"),
        )
    })?;

    // N.B. it looks like autocomplete doesn't work with Discord Users. We could swap this to take a different data type
    // and add autocomplete on *players* behind the scenes. I'm not sure that's better and I also don't feel like implementing it
    // right now.
    let opp_player = match opponent {
        Some(o) => {
            if o == user.id {
                return Ok(UpdateResponseBag::new_content(
                    "Your opponent must be someone other than yourself.",
                ));
            }
            // N.B. we could probably rewrite this to squish the two Player queries into one
            let p = Player::get_by_discord_id(&o.to_string(), cxn.deref_mut()).map_err(|e| {
                ErrorResponse::new(
                    BLAND_USER_FACING_ERROR,
                    format!("Error fetching opponent: {e}"),
                )
            })?;
            match p {
                Some(p) => Some(p),
                None => {
                    return Ok(UpdateResponseBag::new_content(
                        "That opponent could not be found. \
                    Please double check the username and try again.",
                    ));
                }
            }
        }
        None => None,
    };

    let mut races = BracketRace::get_unfinished_races_for_player(&player, cxn.deref_mut())
        .map_err(|e| {
            ErrorResponse::new(
                BLAND_USER_FACING_ERROR,
                format!("Error fetching races for player: {}", e),
            )
        })?;

    fn find_matching_race(vs_whomst: Player, races: Vec<BracketRace>) -> Option<BracketRace> {
        for race in races {
            if race.involves_player(&vs_whomst) {
                return Some(race);
            }
        }
        None
    }

    // Could this be simplified? we are, _in fact_, treating 0, 1, and many differently,
    // but it feels like we should be able to combine the 1 and many cases :thinkDorm:
    let the_race = match races.len() {
        0 => {
            return Ok(UpdateResponseBag::new_content(
                "You do not have an active race.",
            ));
        }
        1 => {
            let candidate = races.pop().unwrap();
            if let Some(opp) = opp_player {
                if candidate.involves_player(&opp) {
                    candidate
                } else {
                    return Ok(UpdateResponseBag::new_content(
                        "Your race is not against that opponent.",
                    ));
                }
            } else {
                candidate
            }
        }
        _ => match opp_player {
            Some(opp) => match find_matching_race(opp, races) {
                Some(r) => r,
                None => {
                    return Ok(UpdateResponseBag::new_content(
                        "No active race found against that opponent.",
                    ));
                }
            },
            None => {
                return Ok(UpdateResponseBag::new_content(
                    "You have more than one active race. Please specify \
                    the opponent you are scheduling against.",
                ));
            }
        },
    };

    match discord::schedule_race(the_race, dt, &state).await {
        Ok(s) => Ok(UpdateResponseBag::new_content(s)),
        Err(ScheduleRaceError::RaceFinished) => Ok(UpdateResponseBag::new_content(
            "Your race for this round is already finished.",
        )),
        Err(e) => Err(ErrorResponse::new(BLAND_USER_FACING_ERROR, e)),
    }
}

fn handle_schedule_race_autocomplete(
    mut ac: Box<CommandData>,
    _interaction: Box<InteractionCreate>,
    _state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    get_focused_opt!("day", &mut ac.options, String).map_err_to_string()?;

    let today = Utc::now().with_timezone(&chrono_tz::US::Eastern);
    let mut options = Vec::with_capacity(7);
    for i in 0..7 {
        let dur = TimeDelta::days(i);
        let day = today.clone() + dur;
        options.push(CommandOptionChoice {
            name: day.format("%A, %B %d").to_string(),
            name_localizations: None,
            value: CommandOptionChoiceValue::String(day.format("%Y/%m/%d").to_string()),
        });
    }
    Ok(autocomplete_result(options))
}

async fn scheduled_races_autocomplete(
    _ac: Box<CommandData>,
    _interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    // TODO: we are simply assuming that this is an autocomplete for the "race" field, because it's annoying to verify right now

    let mut conn = state.diesel_cxn().await.map_err_to_string()?;
    let scheduled_races = BracketRace::scheduled(&mut conn).map_err_to_string()?;
    let mut options = Vec::with_capacity(scheduled_races.len());

    let pids = scheduled_races
        .iter()
        .map(|r| vec![r.player_1_id, r.player_2_id])
        .flatten()
        .collect::<Vec<_>>();
    let involved_players = Player::by_id(Some(pids), &mut conn).map_err_to_string()?;
    let name = |id| {
        involved_players
            .get(&id)
            .map(|p| p.name.as_str())
            .unwrap_or("Unknown")
    };

    for race in scheduled_races {
        let title = {
            let p1 = name(race.player_1_id);
            let p2 = name(race.player_2_id);
            format!("{p1} vs {p2}")
        };

        options.push(CommandOptionChoice {
            name: title,
            name_localizations: None,
            value: CommandOptionChoiceValue::Integer(race.id as i64),
        });
    }
    Ok(autocomplete_result(options))
}

async fn handle_commentator_command(
    mut ac: Box<CommandData>,
    _interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let (cmd_s, mut subcommand_opts) =
        get_subcommand_options(std::mem::take(&mut ac.options)).map_err_to_string()?;
    enum Cmd {
        Add,
        Remove,
    }
    let cmd = match cmd_s.as_str() {
        "add" => Cmd::Add,
        "remove" => Cmd::Remove,
        _ => {
            return Err(format!("Unkown commentator command `{cmd_s}`"));
        }
    };
    let user = get_opt_s!("commentator", &mut subcommand_opts, User)?;
    let race_id = get_opt_s!("race", &mut subcommand_opts, Integer)?;
    let mut conn = state.diesel_cxn().await.map_err_to_string()?;
    let race = BracketRace::get_by_id(race_id as i32, &mut conn).map_err_to_string()?;
    let mut info = race.info(&mut conn).map_err_to_string()?;
    match cmd {
        Cmd::Add => match info
            .new_commentator_signup(user, &mut conn)
            .map_err_to_string()?
        {
            true => {}
            false => {
                return Ok(plain_interaction_response(
                    "That person is already signed up.",
                ));
            }
        },
        Cmd::Remove => match info
            .remove_commentator(user, &mut conn)
            .map_err_to_string()?
        {
            0 => {
                return Ok(plain_interaction_response("That person was not signed up."));
            }
            _ => {}
        },
    }

    // it would be nice to also update the restream request message
    // and maybe send pings to the new comm? but i think it's not very important and it seems annoying to implement

    Ok(plain_interaction_response(format!(
        "Commentators updated on race {race_id}"
    )))
}

async fn handle_set_restream(
    mut ac: Box<CommandData>,
    _interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let race_id = get_opt_s!("race", &mut ac.options, Integer)?;
    let channel = get_opt_s!("channel", &mut ac.options, String)?;
    let mut conn = state.diesel_cxn().await.map_err_to_string()?;
    let race = BracketRace::get_by_id(race_id as i32, &mut conn).map_err_to_string()?;
    let mut info = race.info(&mut conn).map_err_to_string()?;

    if channel == "none" {
        info.restream_channel = None;
    } else {
        match Url::parse(&channel) {
            Ok(p) => {
                if !p.scheme().starts_with("http") {
                    return Ok(plain_interaction_response(
                        "Please provide a full URL including `https://`",
                    ));
                }
                info.restream_channel = Some(channel);
            }
            Err(_e) => {
                return Ok(plain_interaction_response(
                    "Please provide a full URL including `https://`.",
                ));
            }
        }
    }
    info.update(&mut conn).map_err_to_string()?;

    // it would be nice to also update the restream request message
    // and maybe send pings to the new comm? but i think it's not very important and it seems annoying to implement

    Ok(plain_interaction_response(format!(
        "Restream channel updated on {race_id}"
    )))
}

/// people constantly input `name #1234` instead of `name#1234` so let's try to handle that
fn normalize_racetime_name(name: &str) -> String {
    match Regex::new(r"\s+(#\d+)$") {
        Ok(r) => r.replace(name, "$1").to_string(),
        Err(e) => {
            // N.B. the regex should be constructed once, on startup
            info!("Error constructing racetime name regex: {e}");
            name.to_string()
        }
    }
}

async fn handle_update_user_info(
    mut ac: Box<CommandData>,
    mut interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    const BLAND_USER_FACING_ERROR: &str = "Internal error. Sorry.";
    let nickname = get_opt_s!("nickname", &mut ac.options, String).ok();
    let twitch = get_opt_s!("twitch", &mut ac.options, String).ok();
    let racetime = get_opt_s!("racetime", &mut ac.options, String).ok();
    if nickname.is_none() && twitch.is_none() && racetime.is_none() {
        return Ok(Some(plain_interaction_response(
            "You did not provide any information to update.",
        )));
    }
    let user = get_user_from_interaction(&mut interaction).ok_or(ErrorResponse::new(
        BLAND_USER_FACING_ERROR,
        "Unable to find user on handle_update_user_info command",
    ))?;
    let mut cxn = state
        .diesel_cxn()
        .await
        .map_err(|e| ErrorResponse::new(BLAND_USER_FACING_ERROR, e))?;

    let (mut player, _created) = Player::get_or_create_from_discord_user(user, cxn.deref_mut())
        .map_err(|e| ErrorResponse::new(BLAND_USER_FACING_ERROR, e))?;
    let mut save = false;
    let mut user_messages = vec![];
    let mut internal_errors = vec![];
    if let Some(n) = nickname {
        player.name = n;
        save = true;
    }
    // this is somewhat incorrect: we are setting twitch username as long as it's a valid user,
    // and then setting racetime only if the twitch matches; we aren't validating twitch<->discord
    // concordance, nor are we re-validating racetime on subsequent twitch username updates
    // *shrug*
    if let Some(t) = twitch {
        match validate_twitch_username(&t, state).await {
            Ok(Some(normal)) => {
                user_messages.push("Twitch name updated.");
                player.twitch_user_login = Some(normal);
                save = true;
            }
            Ok(None) => {
                user_messages.push("Cannot find that Twitch user.");
            }
            Err(e) => {
                user_messages.push(
                    "There was an error validating your Twitch username. Please try again later.",
                );
                internal_errors.push(e.to_string());
            }
        }
    }
    if let Some(r) = racetime {
        let normalized = normalize_racetime_name(&r);
        match validate_racetime_username(&normalized, player.twitch_user_login.as_ref(), state)
            .await
        {
            Ok(Either::Right(e)) => {
                user_messages.push(e);
            }
            Ok(Either::Left(u)) => {
                user_messages.push("RaceTime name updated.");
                player.racetime_username = Some(normalized);
                player.racetime_user_id = Some(u.id);
                save = true;
            }
            Err(e) => {
                user_messages.push(
                    "There was an error validating your RaceTime username. Please try again later.",
                );
                internal_errors.push(e.to_string());
            }
        }
    }
    if save {
        player
            .update(cxn.deref_mut())
            .map_err(|e| ErrorResponse::new(BLAND_USER_FACING_ERROR, e))?;
    }
    let user_resp = user_messages.join(" ");
    if internal_errors.is_empty() {
        Ok(Some(format_player(
            if user_resp.is_empty() {
                None
            } else {
                Some(user_resp)
            },
            &player,
        )))
    } else {
        Err(ErrorResponse::new(user_resp, internal_errors.join(" ")))
    }
}

async fn handle_user_profile(
    ac: Box<CommandData>,
    _interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    fn get_user(ac: Box<CommandData>) -> Result<User, &'static str> {
        let id = ac
            .target_id
            .ok_or("Missing target_id")?
            .cast::<UserMarker>();
        let mut resolved = ac.resolved.ok_or("Missing resolved")?;
        resolved
            .users
            .remove(&id)
            .ok_or("Target user missing from resolved field")
    }
    let user = get_user(ac).map_err(ErrorResponse::new_internal)?;
    let mut conn = state
        .diesel_cxn()
        .await
        .map_err(ErrorResponse::new_internal)?;

    let (p, _created) = Player::get_or_create_from_discord_user(user.clone(), &mut conn)
        .map_err(ErrorResponse::new_internal)?;

    let formatted = format_player(None, &p);

    Ok(Some(formatted))
}

async fn handle_check_user_info(
    mut _ac: Box<CommandData>,
    mut interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    const BLAND_USER_FACING_ERROR: &str = "Internal error. Sorry.";
    let user = get_user_from_interaction(&mut interaction).ok_or(ErrorResponse::new(
        BLAND_USER_FACING_ERROR,
        "Unable to find user on handle_check_user_info command",
    ))?;
    let mut cxn = state
        .diesel_cxn()
        .await
        .map_err(|e| ErrorResponse::new(BLAND_USER_FACING_ERROR, e))?;

    let (player, _created) = Player::get_or_create_from_discord_user(user, cxn.deref_mut())
        .map_err(|e| ErrorResponse::new(BLAND_USER_FACING_ERROR, e))?;
    Ok(Some(format_player(
        Some(format!(
            "You can use the `/{UPDATE_USER_INFO_CMD}` command to change these properties."
        )),
        &player,
    )))
}

fn format_player(content: Option<String>, player: &Player) -> InteractionResponse {
    let mut fields = vec![EmbedField {
        inline: false,
        name: "Name".to_string(),
        value: player.name.clone(),
    }];
    if let Some(t) = &player.twitch_user_login {
        fields.push(EmbedField {
            inline: false,
            name: "Twitch username".to_string(),
            value: t.clone(),
        });
    }
    if let Some(r) = &player.racetime_username {
        fields.push(EmbedField {
            inline: false,
            name: "RaceTime.gg username".to_string(),
            value: r.clone(),
        });
    }
    let profile_link = format!("{}/player/{}", CONFIG.website_url, player.id,);
    fields.push(EmbedField {
        inline: false,
        name: "Player profile".to_string(),
        value: profile_link,
    });

    let embeds = vec![Embed {
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
        title: Some("Player info".to_string()),
        url: None,
        video: None,
    }];

    InteractionResponse {
        kind: InteractionResponseType::ChannelMessageWithSource,
        data: Some(InteractionResponseData {
            content,
            embeds: Some(embeds),
            flags: Some(MessageFlags::EPHEMERAL),
            ..Default::default()
        }),
    }
}

/// this will attempt to turn `https://twitch.tv/foxlisk` into `foxlisk`
/// if it doesn't look like the first form, it just returns the original string
fn tolerate_twitch_links(link: &str) -> String {
    static URL_REGEX: Lazy<Result<Regex, regex::Error>> = Lazy::new(|| {
        RegexBuilder::new(r"^(?:https?://)?(?:www\.)?twitch.tv/(?<username>\w+?)$")
            .case_insensitive(true)
            .build()
    });
    if let Some(m) = URL_REGEX
        .as_ref()
        .ok()
        .and_then(|r| r.captures(link))
        .and_then(|c| c.name("username"))
    {
        m.as_str().to_string()
    } else {
        link.to_string()
    }
}

/// returns normalized login (i.e. what twitch gives back) if the lookup succeeds, None otherwise
/// This does an API request to twitch
async fn validate_twitch_username(
    username: &str,
    state: &Arc<DiscordState>,
) -> Result<Option<String>, NMGLeagueBotError> {
    let username = tolerate_twitch_links(username);
    let logins = vec![&username];
    let req = GetUsersRequest::logins(logins);
    let mut users = state.twitch_client_bundle.req_get(req).await?;
    // just sort of assuming len == 1 here
    if let Some(u) = users.data.pop() {
        // intellij really thinks this is Iterator::take, but it's actually an aliri_braid method
        let got: String = u.login.take();
        if got.eq_ignore_ascii_case(&username) {
            Ok(Some(got))
        } else {
            Ok(None)
        }
    } else {
        Ok(None)
    }
}

/// term is an rtgg term (lol) of art; it can be like `name` or `#scrim` or `name#scrim`
/// Returns Ok(None) on success, Ok(reason) on logical error, or a behind the scenes error
/// This does an API request to racetime
async fn validate_racetime_username(
    term: &str,
    twitch_login: Option<&String>,
    state: &Arc<DiscordState>,
) -> Result<Either<racetime_api::types::User, &'static str>, NMGLeagueBotError> {
    let us = UserSearch::from_term(term.to_string());
    let UserSearchResult { results } = us.query(&state.racetime_client).await?;
    // if we found any users, make sure we found an exact match (racetime does not offer
    // exact search)
    if let Some(found) = results
        .into_iter()
        .find(|u| u.full_name.eq_ignore_ascii_case(term))
    {
        // if the user we found has a linked twitch channel, *and* they've provided a
        // twitch channel to *us*, we can sanity check
        if let (Some(found_chan), Some(player_login)) = (&found.twitch_channel, twitch_login) {
            if found_chan
                .to_lowercase()
                .ends_with(&format!("/{}", player_login.to_lowercase()))
            {
                Ok(Either::Left(found))
            } else {
                Ok(Either::Right(
                    "The RaceTime account with that name links to a different Twitch account.",
                ))
            }
        } else {
            // if their twitch account isn't linked, we just get to trust them
            Ok(Either::Left(found))
        }
    } else {
        Ok(Either::Right("Cannot find specified RaceTime user."))
    }
}

/// destroys interaction.member
fn get_user_from_interaction(interaction: &mut Box<InteractionCreate>) -> Option<User> {
    std::mem::take(&mut interaction.member)?.user
}

async fn handle_submit_qualifier(
    mut ac: Box<CommandData>,
    mut interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    const BLAND_USER_FACING_ERROR: &str = "Internal error. Sorry.";
    let user = get_user_from_interaction(&mut interaction).ok_or(ErrorResponse::new(
        BLAND_USER_FACING_ERROR,
        "Unable to find user on handle_submit_qualifier command",
    ))?;

    let time = match get_opt_s!("qualifier_time", &mut ac.options, String) {
        Ok(t) => t,
        Err(e) => {
            return Ok(Some(plain_interaction_response(e)));
        }
    };
    let secs = match utils::parse_hms(&time) {
        Some(t) => t,
        None => {
            return Ok(Some(plain_interaction_response(
                "Invalid time. Please use h:mm:ss format.",
            )));
        }
    };
    let vod = match get_opt_s!("vod", &mut ac.options, String) {
        Ok(v) => v,
        Err(e) => {
            return Ok(Some(plain_interaction_response(e)));
        }
    };

    let mut cxn = state
        .diesel_cxn()
        .await
        .map_err(|e| ErrorResponse::new(BLAND_USER_FACING_ERROR, e))?;

    let (player, created) = match Player::get_or_create_from_discord_user(user, cxn.deref_mut()) {
        Ok(p) => p,
        Err(e) => {
            return Err(ErrorResponse::new(BLAND_USER_FACING_ERROR, e));
        }
    };

    let update_pls_suffix = if created {
        format!(" I would really appreciate it if you would run the `/{UPDATE_USER_INFO_CMD}` command and provide your \
         Twitch and RaceTime info!")
    } else {
        "".to_string()
    };
    let current_season = match active_season_with_quals_open(cxn.deref_mut()) {
        Ok(Some(s)) => s,
        Ok(None) => {
            return Ok(Some(plain_interaction_response(format!(
                "Qualifiers are not currently open.{update_pls_suffix}"
            ))));
        }
        Err(e) => {
            return Err(ErrorResponse::new(BLAND_USER_FACING_ERROR, e));
        }
    };
    let nqs = NewQualifierSubmission::new(&player, &current_season, secs, vod);
    nqs.save(cxn.deref_mut())
        .map(|_| {
            Some(plain_interaction_response(format!(
                "Thanks for your submission!{update_pls_suffix}"
            )))
        })
        .map_err(|e| ErrorResponse::new(BLAND_USER_FACING_ERROR, e))
}

fn active_season_with_quals_open(
    cxn: &mut SqliteConnection,
) -> Result<Option<Season>, NMGLeagueBotError> {
    match Season::get_active_season(cxn)? {
        None => Ok(None),
        Some(s) => {
            if s.are_qualifiers_open()? {
                Ok(Some(s))
            } else {
                Ok(None)
            }
        }
    }
}

async fn handle_reschedule_race_cmd(
    ac: Box<CommandData>,
    _interaction: Box<InteractionCreate>,
    state: Arc<DiscordState>,
) -> Result<UpdateResponseBag, ErrorResponse> {
    Ok(match _handle_reschedule_race_cmd(ac, state).await {
        Ok(u) => u,
        Err(e) => UpdateResponseBag::new_content(e),
    })
}

async fn _handle_reschedule_race_cmd(
    mut ac: Box<CommandData>,
    state: Arc<DiscordState>,
) -> Result<UpdateResponseBag, String> {
    let race_id = get_opt_s!("race_id", &mut ac.options, Integer)?;
    let mut cxn = state.diesel_cxn().await.map_err_to_string()?;
    let race = match BracketRace::get_by_id(race_id as i32, cxn.deref_mut()) {
        Ok(br) => br,
        Err(Error::NotFound) => {
            return Err(format!("Race #{race_id} not found."));
        }
        Err(e) => {
            return Err(format!("{e}"));
        }
    };
    let when = get_datetime_from_scheduling_cmd(&mut ac.options).map_err_to_string()?;
    discord::schedule_race(race, when, &state)
        .await
        .map(|s| UpdateResponseBag::new_content(s))
        .map_err_to_string()
}

async fn handle_create_player(
    mut ac: Box<CommandData>,
    _interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let discord_user = get_opt_s!("user", &mut ac.options, User)?;
    let rt_un = get_opt_s!("rtgg_username", &mut ac.options, String)?;
    let twitch_name = get_opt_s!("twitch_username", &mut ac.options, String)?;
    let name_override = get_opt_s!("name", &mut ac.options, String).ok();
    let name = match name_override {
        Some(name) => name,
        None => {
            let u = state.get_user(discord_user);
            u.ok_or(format!("User not found"))?.name
        }
    };
    let mut cxn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    // TODO: do some validation/lookups here? or centralize that logic?
    let np = NewPlayer::new(
        name,
        discord_user.to_string(),
        Some(rt_un),
        Some(twitch_name),
        None,
    );

    match np.save(cxn.deref_mut()) {
        Ok(_) => Ok(plain_interaction_response("Player added!")),
        Err(e) => Err(e.to_string()),
    }
}

async fn handle_add_player_to_bracket(
    ac: Box<CommandData>,
    interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    match interaction.kind {
        InteractionType::ApplicationCommand => {
            handle_add_player_to_bracket_submit(ac, interaction, state).await
        }
        InteractionType::ApplicationCommandAutocomplete => {
            handle_add_player_to_bracket_autocomplete(ac, interaction, state).await
        }
        _ => Err(format!(
            "Unexpected InteractionType for {}",
            ADD_PLAYER_TO_BRACKET_CMD
        )),
    }
}

async fn handle_add_player_to_bracket_submit(
    mut ac: Box<CommandData>,
    _interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let discord_id = get_opt_s!("user", &mut ac.options, User)?;
    let bracket_name = get_opt_s!("bracket", &mut ac.options, String)?;
    let mut cxn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    let user = state.get_user(discord_id).ok_or("Cannot find that user")?;

    let (player, created) = match Player::get_or_create_from_discord_user(user, cxn.deref_mut()) {
        Ok(p) => p,
        Err(e) => {
            return Err(format!("Error getting or creating player: {e}"));
        }
    };

    let update_pls_suffix = if created {
        format!(" This player was freshly created and has no user info!")
    } else {
        "".to_string()
    };

    let szn = Season::get_active_season(cxn.deref_mut())
        .map_err(|e| e.to_string())?
        .ok_or("There's no active season.".to_string())?;
    let bracket = szn
        .brackets(cxn.deref_mut())
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|b| b.name == bracket_name)
        .ok_or(format!(
            "Cannot find bracket {} in Season {}",
            bracket_name, szn.ordinal
        ))?;

    let npbe = NewPlayerBracketEntry::new(&bracket, &player);
    npbe.save(cxn.deref_mut())
        .map(|_| {
            plain_interaction_response(format!(
                "{} added to {}.{update_pls_suffix}",
                player.name, bracket.name
            ))
        })
        .map_err(|e| e.to_string())
}

async fn get_bracket_autocompletes(
    mut ac: Box<CommandData>,
    state: &Arc<DiscordState>,
) -> Result<Vec<CommandOptionChoice>, String> {
    get_focused_opt!("bracket", &mut ac.options, String).map_err_to_string()?;

    let mut cxn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    let szn = Season::get_active_season(cxn.deref_mut())
        .map_err(|e| e.to_string())?
        .ok_or("No current season!!!".to_string())?;
    let brackets = szn.brackets(cxn.deref_mut()).map_err(|e| e.to_string())?;
    Ok(brackets
        .into_iter()
        .map(|b| CommandOptionChoice {
            name: b.name.clone(),
            name_localizations: None,
            value: CommandOptionChoiceValue::String(b.name),
        })
        .collect())
}

async fn handle_add_player_to_bracket_autocomplete(
    ac: Box<CommandData>,
    _interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    // just to validate that we're autocompleting what we think we are.
    let options = match get_bracket_autocompletes(ac, state).await {
        Ok(o) => o,
        Err(e) => {
            warn!("Error fetching bracket autocompletes: {}", e);
            vec![]
        }
    };
    Ok(InteractionResponse {
        kind: InteractionResponseType::ApplicationCommandAutocompleteResult,
        data: Some(InteractionResponseData {
            allowed_mentions: None,
            attachments: None,
            choices: Some(options),
            components: None,
            content: None,
            custom_id: None,
            embeds: None,
            flags: None,
            title: None,
            tts: None,
            poll: None,
        }),
    })
}

async fn handle_create_race(
    mut ac: Box<CommandData>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let p1 = get_opt_s!("p1", &mut ac.options, User)?;
    let p2 = get_opt_s!("p2", &mut ac.options, User)?;
    let on_start_message = get_opt_s!("on_start_message", &mut ac.options, String).ok();

    if p1 == p2 {
        return Ok(plain_interaction_response(
            "The racers must be different users",
        ));
    }

    let new_race = NewAsyncRace::new(on_start_message);
    let mut cxn = state
        .diesel_cxn()
        .await
        .map_err(|e| format!("Error getting database connection: {}", e))?;
    let race = new_race
        .save(cxn.deref_mut())
        .map_err(|e| format!("Error saving race: {}", e))?;

    let (mut r1, mut r2) = race
        .select_racers(p1.clone(), p2.clone(), &mut cxn)
        .await
        .map_err(|e| format!("Error saving race runs: {}", e))?;

    let (first, second) = {
        tokio::join!(
            notify_racer(&mut r1, &race, &state),
            notify_racer(&mut r2, &race, &state)
        )
    };
    // this is annoying, i havent really found a pattern i like for "report 0-2 errors" in Rust yet
    match (first, second) {
        (Ok(_), Ok(_)) => Ok(plain_interaction_response(format!(
            "Race #{} created for users {} and {}",
            race.id,
            p1.mention(),
            p2.mention(),
        ))),
        (Err(e), Ok(_)) => Ok(plain_interaction_response(format!(
            "Error creating race: error contacting {}: {}",
            p1.mention(),
            e
        ))),
        (Ok(_), Err(e)) => Ok(plain_interaction_response(format!(
            "Error creating race: error contacting {}: {}",
            p2.mention(),
            e
        ))),
        (Err(e1), Err(e2)) => Ok(plain_interaction_response(format!(
            "Error creating race: error contacting {}: {} \
            error contacting {}: {}",
            p1.mention(),
            e1,
            p2.mention(),
            e2
        ))),
    }
}

async fn handle_cancel_race(
    mut ac: Box<CommandData>,
    interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, String> {
    let race_id = get_opt_s!("race_id", &mut ac.options, Integer)?;

    if !ac.options.is_empty() {
        return Err(format!(
            "I'm very confused: {} had an unexpected option",
            CANCEL_ASYNC_CMD
        ));
    }

    let mut conn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    let race = match AsyncRace::get_by_id(race_id as i32, &mut conn) {
        Ok(r) => r,
        Err(_e) => {
            return Ok(Some(plain_interaction_response(
                "Cannot find a race with that ID",
            )));
        }
    };

    if race.state != RaceState::CREATED {
        return Ok(Some(plain_interaction_response(format!(
            "It does not make sense to me to cancel a race in state {}",
            String::from(race.state)
        ))));
    }

    let (r1, r2) = match AsyncRaceRun::get_runs(&race, &mut conn).await {
        Ok(rs) => rs,
        Err(e) => {
            return Ok(Some(plain_interaction_response(format!(
                "Unable to find runs associated with that race: {}",
                e
            ))));
        }
    };

    if !r1.state.is_pre_start() || !r2.state.is_pre_start() {
        handle_cancel_race_started(interaction, race, r1, r2, state)
            .await
            .ok();
        Ok(None)
    } else {
        actually_cancel_race(race, r1, r2, state)
            .await
            .map(|_| Some(plain_interaction_response("Race cancelled.")))
    }
}

// this method returns () because it is taking over the interaction flow. we're adding a new
// interaction cycle and not operating on the original interaction anymore.
async fn handle_cancel_race_started(
    ac: Box<InteractionCreate>,
    race: AsyncRace,
    r1: AsyncRaceRun,
    r2: AsyncRaceRun,
    state: &Arc<DiscordState>,
) -> Result<(), String> {
    let mut resp =
        plain_interaction_response("Are you sure? One of those runs has already been started.");
    if let Some(ref mut d) = resp.data {
        d.components = Some(action_row(vec![
            button_component("Really cancel race", REALLY_CANCEL_ID, ButtonStyle::Danger),
            button_component("Do not cancel race", "dont_cancel", ButtonStyle::Secondary),
        ]));
    }
    state
        .create_response_err_to_str(ac.id.clone(), &ac.token, &resp)
        .await?;
    let msg_resp = state
        .interaction_client()
        .response(&ac.token)
        .await
        .map_err(|e| format!("Error asking you if you were serious? lol what: {}", e))?;
    let msg = msg_resp
        .model()
        .await
        .map_err(|e| format!("Error deserializing response: {}", e))?;

    match wait_for_cancel_race_decision(msg.id, state).await {
        Ok(cmp) => {
            // if we got a button click we have to deal with that interaction, specifically via
            // creating an "update response"
            let cid = interaction_to_custom_id(&cmp);
            let resp = match cid {
                Some(REALLY_CANCEL_ID) => actually_cancel_race(race, r1, r2, state)
                    .await
                    .map(|()| "Race cancelled.".to_string())
                    .collapse(),
                Some(_) => "Okay, not cancelling it.".to_string(),
                None => "Not cancelling it with a side of bizarre internal error.".to_string(),
            };
            state
                .create_response_err_to_str(cmp.id, &cmp.token, &update_resp_to_plain_content(resp))
                .await
        }
        Err(e) => {
            // otherwise (some kind of timeout or other error) we update the last interaction
            state
                .interaction_client()
                .update_response(&ac.token)
                .components(Some(&[]))
                .content(Some(&e))
                .await
                .map_err(|e| format!("Error updating message: {}", e))
                .map(|_| ())
        }
    }
}

/// returns the new component interaction if the user indicates their choice, otherwise
/// an error indicating what happened instead.
async fn wait_for_cancel_race_decision(
    mid: Id<MessageMarker>,
    state: &Arc<DiscordState>,
) -> Result<Interaction, String> {
    let sb = state.standby.wait_for_component(
        mid,
        // I don't know why but spelling out the parameter type here seems to fix a compiler
        // complaint
        |_: &Interaction| true,
    );

    match tokio::time::timeout(tokio::time::Duration::from_secs(CONFIG.cancel_race_timeout), sb).await {
        Ok(cmp) => {
            cmp.map_err(|c| format!("Weird internal error to do with dropping a Standby: {:?}", c))
        }
        Err(_timeout) => {
            Err(format!("This cancellation has timed out, please re-run the command if you still want to cancel."))
        }
    }
}

async fn actually_cancel_race(
    race: AsyncRace,
    r1: AsyncRaceRun,
    r2: AsyncRaceRun,
    state: &Arc<DiscordState>,
) -> Result<(), String> {
    let mut conn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    race.cancel(&mut conn)
        .await
        .map_err(|e| format!("Error cancelling race: {}", e))?;

    let (r1_update, r2_update) = tokio::join!(
        update_cancelled_race_message(r1, state),
        update_cancelled_race_message(r2, state),
    );

    let errors = r1_update
        .err()
        .into_iter()
        .chain(r2_update.err().into_iter())
        .collect::<Vec<String>>();
    if !errors.is_empty() {
        return Err(format!(
            "Error updating messages to racers: {}",
            errors.join("; ")
        ));
    }

    Ok(())
}

async fn update_cancelled_race_message(
    run: AsyncRaceRun,
    state: &Arc<DiscordState>,
) -> Result<(), String> {
    let mid = run
        .get_message_id()
        .ok_or(format!("Unable to find message associated with run"))?;
    let cid = state.get_private_channel(run.racer_id()?).await?;
    let update = update_interaction_message_to_plain_text(
        mid,
        cid,
        "This race has been cancelled by an admin.",
        state,
    );
    update
        .await
        .map_err(|e| format!("Error updating race run message: {}", e))
        .map(|_| ())
}

// this should be called something like "race_cancelled_update_message" since it's building an
// "UpdateMessage" object to indicate "Race Cancelled" but that just sounds like word salad in
// combination with update_cancelled_race_message
fn update_interaction_message_to_plain_text<'a>(
    mid: Id<MessageMarker>,
    cid: Id<ChannelMarker>,
    text: &'a str,
    state: &'a Arc<DiscordState>,
) -> UpdateMessage<'a> {
    state
        .discord_client
        .update_message(cid, mid)
        .attachments(&[])
        .components(Some(&[]))
        .embeds(Some(&[]))
        .content(Some(text))
}

async fn handle_set_season_state(
    mut ac: Box<CommandData>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let season_ordinal = get_opt_s!("season_ordinal", &mut ac.options, Integer)?;
    let new_state_raw = get_opt_s!("new_state", &mut ac.options, String)?;
    let new_state: SeasonState =
        serde_json::from_str(&new_state_raw).map_err(|e| format!("Error parsing state: {e}"))?;
    let mut cxn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    let mut season =
        Season::get_by_ordinal(season_ordinal as i32, cxn.deref_mut()).map_err_to_string()?;
    season
        .set_state(new_state, cxn.deref_mut())
        .map_err_to_string()?;
    season.update(cxn.deref_mut()).map_err_to_string()?;
    Ok(plain_interaction_response("Update successful."))
}

async fn handle_create_season(
    mut ac: Box<CommandData>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let format = get_opt_s!("format", &mut ac.options, String)?;
    let category = get_opt_s!("rtgg_category_name", &mut ac.options, String)?;
    let goal = get_opt_s!("rtgg_goal_name", &mut ac.options, String)?;
    let mut cxn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    let ns = NewSeason::new(format, category, goal, cxn.deref_mut()).map_err_to_string()?;

    let s = ns.save(cxn.deref_mut()).map_err(|e| e.to_string())?;
    Ok(plain_interaction_response(format!(
        "Season {} created!",
        s.ordinal
    )))
}

async fn handle_see_unscheduled_races(
    _ac: Box<CommandData>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let mut cxn = state.diesel_cxn().await.map_err(|e| e.to_string())?;

    let mut unscheduled = BracketRace::unscheduled(cxn.deref_mut()).map_err_to_string()?;
    if unscheduled.is_empty() {
        return Ok(plain_ephemeral_response("There are no unscheduled races."));
    }
    unscheduled.sort_by_key(|br| br.bracket_id);
    let mut fields = vec![];
    for br in unscheduled {
        let (p1, p2) = br.players(cxn.deref_mut()).map_err_to_string()?;
        let bname = br.bracket(cxn.deref_mut()).map_err_to_string()?.name;
        let field = EmbedField {
            inline: false,
            name: format!("{} vs {}", p1.name, p2.name),
            value: bname,
        };
        fields.push(field);
    }
    let ir = InteractionResponse {
        kind: InteractionResponseType::ChannelMessageWithSource,
        data: Some(InteractionResponseData {
            embeds: Some(vec![Embed {
                fields,
                author: None,
                color: None,
                description: None,
                footer: None,
                image: None,
                kind: "rich".to_string(),
                provider: None,
                thumbnail: None,
                timestamp: None,
                title: None,
                url: None,
                video: None,
            }]),
            ..Default::default()
        }),
    };
    Ok(ir)
}

async fn handle_create_bracket(
    mut ac: Box<CommandData>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let name = get_opt_s!("name", &mut ac.options, String)?;
    let bracket_type = get_opt_s!("bracket_type", &mut ac.options, String)?;
    let bt: BracketType = serde_json::from_str(&bracket_type).map_err_to_string()?;
    let mut conn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    let szn = Season::get_active_season(conn.deref_mut())
        .and_then(|os| os.ok_or(diesel::result::Error::NotFound))
        .map_err_to_string()?;
    let nb = NewBracket::new(&szn, name, bt);
    nb.save(conn.deref_mut()).map_err(|e| e.to_string())?;
    Ok(plain_interaction_response("Bracket created!"))
}

async fn handle_finish_bracket(
    mut ac: Box<CommandData>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let bracket_id = get_opt_s!("bracket_id", &mut ac.options, Integer)?;
    let mut conn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    let mut bracket =
        Bracket::get_by_id(bracket_id as i32, conn.deref_mut()).map_err_to_string()?;
    let resp = if bracket.finish(conn.deref_mut()).map_err_to_string()? {
        bracket.update(conn.deref_mut()).map_err_to_string()?;
        "Bracket finished."
    } else {
        "Unable to finish that bracket (round still in progress)."
    };
    Ok(plain_interaction_response(resp))
}

// wow dude great function name
async fn get_race_finish_opts_from_command_opts(
    options: &mut Vec<CommandDataOption>,
    state: &Arc<DiscordState>,
    force: bool,
) -> Result<RaceFinishOptions, String> {
    let race_id = get_opt_s!("race_id", options, Integer)?;
    let p1_res = get_opt_s!("p1_result", options, String)?;
    let p2_res = get_opt_s!("p2_result", options, String)?;
    let racetime_url = get_opt_s!("racetime_url", options, String).ok();
    let r1 = parse_race_result(&p1_res).map_err_to_string()?;

    let r2 = parse_race_result(&p2_res).map_err_to_string()?;
    let mut cxn = state.diesel_cxn().await.map_err_to_string()?;
    let race = match BracketRace::get_by_id(race_id as i32, cxn.deref_mut()) {
        Ok(r) => r,
        Err(Error::NotFound) => {
            return Err("That race ID does not exist".to_string());
        }
        Err(e) => {
            return Err(format!("Other database error: {e}"));
        }
    };
    let mut info = race.info(cxn.deref_mut()).map_err_to_string()?;
    if let Some(rt) = racetime_url {
        info.racetime_gg_url = Some(rt);
    }
    let (p1, p2) = race.players(cxn.deref_mut()).map_err_to_string()?;
    Ok(RaceFinishOptions {
        bracket_race: race,
        info,
        player_1: p1,
        player_1_result: r1,
        player_2: p2,
        player_2_result: r2,
        channel_id: state.channel_config.match_results,
        force_update: force,
    })
}

async fn handle_report_race(
    mut ac: Box<CommandData>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let opts = get_race_finish_opts_from_command_opts(&mut ac.options, state, false).await?;
    let mut cxn = state.diesel_cxn().await.map_err_to_string()?;
    trigger_race_finish(opts, cxn.deref_mut(), Some(&state.discord_client),  &state.channel_config)
        .await
        .map(|_|plain_interaction_response(format!(
            "Race has been updated. You should see a post in {}",
            state.channel_config.match_results.mention()
        )))
        .map_err(|e| match e {
            RaceFinishError::BracketRaceStateError(BracketRaceStateError::InvalidState(_, _)) => {
                format!("That race is already finished. Please use `/{UPDATE_FINISHED_RACE_CMD}` if you are trying to \
                change the results of a finished race.")
            }
            e => e.to_string()
        })
}

async fn handle_rereport_race(
    mut ac: Box<CommandData>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let opts = get_race_finish_opts_from_command_opts(&mut ac.options, state, true).await?;
    if opts.bracket_race.state().map_err_to_string()? != BracketRaceState::Finished {
        return Err(format!(
            "That race is not yet reported. Please use `/{REPORT_RACE_CMD}` if you are trying to \
            report an unfinished race."
        ));
    }
    let mut cxn = state.diesel_cxn().await.map_err_to_string()?;
    trigger_race_finish(
        opts,
        cxn.deref_mut(),
        Some(&state.discord_client),
        &state.channel_config,
    )
    .await
    .map(|_| {
        plain_interaction_response(format!(
            "Race has been updated. You should see a post in {}",
            state.channel_config.match_results.mention()
        ))
    })
    .map_err_to_string()
}

async fn handle_generate_pairings(
    mut ac: Box<CommandData>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let bracket_id = get_opt_s!("bracket_id", &mut ac.options, Integer)?;
    let mut cxn = state.diesel_cxn().await.map_err_to_string()?;
    let mut b = match Bracket::get_by_id(bracket_id as i32, cxn.deref_mut()) {
        Ok(b) => b,
        Err(Error::NotFound) => {
            return Err(format!("Bracket {bracket_id} not found."));
        }
        Err(e) => {
            return Err(e.to_string());
        }
    };
    // i hate this for a couple reasons, but I am pacifying myself by remembering that a query of a sqlite table
    // with 6 rows is not actually a big performance issue
    let szn = Season::get_by_id(b.season_id, cxn.deref_mut()).map_err_to_string()?;

    let url = crate::uri!(bracket_detail(
        season_ordinal = szn.ordinal,
        bracket_id = b.id
    ));
    match b.generate_pairings(cxn.deref_mut()) {
        Ok(()) => Ok(plain_interaction_response(format!(
            "Pairings generated! See them at {}{url}",
            CONFIG.website_url,
        ))),
        Err(e) => Err(format!("Error generating pairings: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use crate::discord::interaction_handlers::application_commands::{
        datetime_from_options, normalize_racetime_name, tolerate_twitch_links,
    };
    use chrono::{Datelike, Timelike};

    #[test]
    fn test_datetime_thingy() {
        let thing = datetime_from_options("2022/10/28", 12, 13, "AM").unwrap();
        assert_eq!(thing.day(), 28);
        assert_eq!(thing.month(), 10);
        assert_eq!(thing.hour(), 0);
        assert_eq!(thing.format("%v %r").to_string(), "28-Oct-2022 12:13:00 AM");

        let other_thing = datetime_from_options("2022/10/28", 12, 13, "PM").unwrap();
        assert_eq!(other_thing.day(), 28);
        assert_eq!(other_thing.month(), 10);
        assert_eq!(other_thing.hour(), 12);
        assert_eq!(
            other_thing.format("%v %r").to_string(),
            "28-Oct-2022 12:13:00 PM"
        );
    }

    #[test]
    fn test_normalize_racetime_name() {
        let already_good = "foxlisk#1234";
        assert_eq!(already_good, normalize_racetime_name(already_good));

        let stray_space = "foxlisk #1234";
        assert_eq!("foxlisk#1234", normalize_racetime_name(stray_space));

        let weird_spaces_already_good = "a b c#1234";
        assert_eq!(
            weird_spaces_already_good,
            normalize_racetime_name(weird_spaces_already_good)
        );

        let weird_spaces_stray_space = "a b c    #1234";
        assert_eq!(
            "a b c#1234",
            normalize_racetime_name(weird_spaces_stray_space)
        );
    }

    #[test]
    fn test_twitch_links() {
        for (input, expected) in vec![
            ("foxlisk", "foxlisk"),
            ("https://twitch.tv/foxlisk", "foxlisk"),
            ("twitch.tv/foxlisk", "foxlisk"),
            ("http://twitch.tv/asdf_12345", "asdf_12345"),
            ("https://www.twitch.tv/shkoople", "shkoople"),
            ("www.twitch.tv/shkoople", "shkoople"),
            ("Twitch.tv/Cen7ipede", "Cen7ipede"),
        ] {
            assert_eq!(tolerate_twitch_links(input), expected.to_string())
        }
    }
}
