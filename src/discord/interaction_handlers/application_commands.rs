use crate::constants::CANCEL_RACE_TIMEOUT_VAR;
use crate::discord::components::action_row;
use crate::discord::discord_state::DiscordState;
use crate::discord::interactions::{
    button_component, interaction_to_custom_id, plain_interaction_response,
    update_resp_to_plain_content,
};
use crate::discord::{notify_racer, ErrorResponse, ADD_PLAYER_TO_BRACKET_CMD, CANCEL_RACE_CMD, CREATE_BRACKET_CMD, CREATE_PLAYER_CMD, CREATE_RACE_CMD, CREATE_SEASON_CMD, get_opt};
use crate::get_opt;
use crate::models::race::{NewRace, Race, RaceState};
use crate::models::race_run::RaceRun;
use diesel::{QueryResult, RunQueryDsl};
use nmg_league_bot::models::brackets::{Bracket, NewBracket};
use nmg_league_bot::models::player::{NewPlayer, Player};
use nmg_league_bot::models::player_bracket_entries::NewPlayerBracketEntry;
use nmg_league_bot::models::player_bracket_entries::PlayerBracketEntry;
use nmg_league_bot::models::season::{NewSeason, Season};
use nmg_league_bot::utils::{env_default, ResultCollapse};
use std::ops::DerefMut;
use std::sync::Arc;
use twilight_http::request::channel::message::UpdateMessage;
use twilight_mention::Mention;
use twilight_model::application::command::{CommandOptionChoice, CommandOptionType};
use twilight_model::application::component::button::ButtonStyle;
use twilight_model::application::interaction::application_command::{
    CommandData, CommandOptionValue,
};
use twilight_model::application::interaction::{Interaction, InteractionType};
use twilight_model::gateway::payload::incoming::InteractionCreate;
use twilight_model::http::interaction::{
    InteractionResponse, InteractionResponseData, InteractionResponseType,
};
use twilight_model::id::marker::{ChannelMarker, MessageMarker};
use twilight_model::id::Id;
use twilight_validate::message::MessageValidationError;

const REALLY_CANCEL_ID: &'static str = "really_cancel";

/// this doesn't have an option to return an ErrorResponse because these interactions already occur
/// under the watchful eyes of admins (and are, in fact, run _by_ admins)
///
/// N.B. interaction.data is already ripped out, here, and is passed in as the first parameter
pub async fn handle_application_interaction(
    ac: Box<CommandData>,
    interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    match ac.name.as_str() {
        // general commands
        // there aren't any right now... but if there are we just stick them in here,
        // return whatever we find, and if we find nothing we validate adminniness and move on
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
    match ac.name.as_str() {
        CREATE_PLAYER_CMD => {
            admin_command_wrapper(create_player(ac, interaction, state).await.map(|i| Some(i)))
        }
        ADD_PLAYER_TO_BRACKET_CMD => {
            println!("handle add data: interaction: {:?}", interaction);
            admin_command_wrapper(
                handle_add_player_to_bracket(ac, interaction, state)
                    .await
                    .map(|i| Some(i)),
            )
        }
        // admin commands
        CREATE_RACE_CMD => {
            admin_command_wrapper(handle_create_race(ac, state).await.map(|i| Some(i)))
        }

        CANCEL_RACE_CMD => admin_command_wrapper(handle_cancel_race(ac, interaction, state).await),
        CREATE_SEASON_CMD => {
            admin_command_wrapper(handle_create_season(ac, state).await.map(|i| Some(i)))
        }

        CREATE_BRACKET_CMD => {
            admin_command_wrapper(handle_create_bracket(ac, state).await.map(|i| Some(i)))
        }

        _ => {
            println!("Unhandled application command: {}", ac.name);
            Ok(None)
        }
    }
}

/// turns a "String" error response into a plain interaction response with that text
///
/// designed for use on admin-only commands, where errors should just be reported to the admins
fn admin_command_wrapper(
    result: Result<Option<InteractionResponse>, String>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    let out = Ok(result
        .map_err(|e| Some(plain_interaction_response(e)))
        .collapse());
    println!("admin_command_wrapper: returning {:?}", out);
    out
}

async fn create_player(
    mut ac: Box<CommandData>,
    _interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let discord_user = get_opt!("user", &mut ac.options, User)?;
    let rt_un = get_opt!("rtgg_username", &mut ac.options, String)?;
    let name_override = get_opt!("name", &mut ac.options, String).ok();
    let name = match name_override {
        Some(name) => name,
        None => {
            let u = state
                .get_user(discord_user)
                .await
                .map_err(|e| format!("Error finding user??? {}", e))?;
            u.ok_or(format!("User not found"))?.name
        }
    };
    let mut cxn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    let np = NewPlayer::new(name, discord_user.to_string(), rt_un, true);

    match np.save(cxn.deref_mut()) {
        Ok(_) => Ok(plain_interaction_response("Player added!")),
        Err(e) => Err(e.to_string()),
    }
}

async fn handle_add_player_to_bracket(
    mut ac: Box<CommandData>,
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
    interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let discord_id = get_opt!("user", &mut ac.options, User)?;
    let bracket_name = get_opt!("bracket", &mut ac.options, String)?;
    let mut cxn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    let player = match Player::get_by_discord_id(&(discord_id.to_string()), cxn.deref_mut())
        .map_err(|e| e.to_string())?
    {
        Some(p) => p,
        None => {
            return Ok(plain_interaction_response(format!(
                "That player has not been created. Use /{} to create them.",
                ADD_PLAYER_TO_BRACKET_CMD
            )));
        }
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
            bracket_name, szn.id
        ))?;

    let npbe = NewPlayerBracketEntry::new(&bracket, &player);
    npbe.save(cxn.deref_mut())
        .map(|_| plain_interaction_response(format!("{} added to {}", player.name, bracket.name)))
        .map_err(|e| e.to_string())
}

async fn get_bracket_autocompletes(mut ac: Box<CommandData>,
                                   state: &Arc<DiscordState>,) -> Result<Vec<CommandOptionChoice>, String> {
    let b = get_opt("bracket", &mut ac.options, CommandOptionType::String)?;
    if let CommandOptionValue::Focused(_, _) = b.value {
        // this is what we're expecting
    } else {
        println!("Found option {:?}", b);
        return Err("Trying to autocomplete for bracket but it's not the bracket arg".to_string());
    }
    let mut cxn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    let szn = Season::get_active_season(cxn.deref_mut())
        .map_err(|e| e.to_string())?
        .ok_or("No current season!!!".to_string())?;
    let brackets = szn.brackets(cxn.deref_mut()).map_err(|e| e.to_string())?;
    Ok(brackets
        .into_iter()
        .map(|b| CommandOptionChoice::String {
            name: b.name.clone(),
            name_localizations: None,
            value: b.name,
        })
        .collect())

}

async fn handle_add_player_to_bracket_autocomplete(
    mut ac: Box<CommandData>,
    interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    // just to validate that we're autocompleting what we think we are.
    let options = match get_bracket_autocompletes(ac, state).await {
        Ok(o) => o,
        Err(e) => {
            println!("Error fetching bracket autocompletes: {}", e);
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
        }),
    })
}

async fn handle_create_race(
    mut ac: Box<CommandData>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    // twilight_model::application::command::CommandOptionValue::
    let p1 = get_opt!("p1", &mut ac.options, User)?;
    let p2 = get_opt!("p2", &mut ac.options, User)?;

    if p1 == p2 {
        return Ok(plain_interaction_response(
            "The racers must be different users",
        ));
    }

    let new_race = NewRace::new();
    let mut cxn = state
        .diesel_cxn()
        .await
        .map_err(|e| format!("Error getting database connection: {}", e))?;

    let race: Race = diesel::insert_into(crate::schema::races::table)
        .values(new_race)
        .get_result(cxn.deref_mut())
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
    let race_id = get_opt!("race_id", &mut ac.options, Integer)?;

    if !ac.options.is_empty() {
        return Err(format!(
            "I'm very confused: {} had an unexpected option",
            CANCEL_RACE_CMD
        ));
    }

    let mut conn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    let race = match Race::get_by_id(race_id as i32, &mut conn) {
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

    let (r1, r2) = match RaceRun::get_runs(race.id, &mut conn).await {
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
    race: Race,
    r1: RaceRun,
    r2: RaceRun,
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
        .exec()
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
                .and_then(|c| c.content(Some(&e)))
                .map_err(|validation_error| {
                    format!("Error building message: {}", validation_error)
                })?
                .exec()
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

    let time = env_default(CANCEL_RACE_TIMEOUT_VAR, 90);
    match tokio::time::timeout(tokio::time::Duration::from_secs(time), sb).await {
        Ok(cmp) => {
            cmp.map_err(|c| format!("Weird internal error to do with dropping a Standby: {:?}", c))
        }
        Err(_timeout) => {
            Err(format!("This cancellation has timed out, please re-run the command if you still want to cancel."))
        }
    }
}

async fn actually_cancel_race(
    race: Race,
    r1: RaceRun,
    r2: RaceRun,
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
    run: RaceRun,
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
    )
    .map_err(|e| e.to_string())?;
    update
        .exec()
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
) -> Result<UpdateMessage<'a>, MessageValidationError> {
    state
        .client
        .update_message(cid, mid)
        .attachments(&[])?
        .components(Some(&[]))?
        .embeds(Some(&[]))?
        .content(Some(text))
}

async fn handle_create_season(
    mut ac: Box<CommandData>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let format = get_opt!("format", &mut ac.options, String)?;

    let ns = NewSeason::new(format);
    let mut cxn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    let s = ns.save(cxn.deref_mut()).map_err(|e| e.to_string())?;
    Ok(plain_interaction_response(format!("Season {} created!", s.id)))
}

async fn handle_create_bracket(
    mut ac: Box<CommandData>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let name = get_opt!("name", &mut ac.options, String)?;
    let season_id = get_opt!("season_id", &mut ac.options, Integer)?;
    let mut conn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    // TODO: look up season, save thing, whatever
    let szn = Season::get_by_id(season_id as i32, conn.deref_mut())?;
    let nb = NewBracket::new(&szn, name);
    nb.save(conn.deref_mut()).map_err(|e| e.to_string())?;
    Ok(plain_interaction_response("Bracket created!"))
}
