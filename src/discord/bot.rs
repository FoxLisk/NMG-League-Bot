use crate::constants::{APPLICATION_ID_VAR, CANCEL_RACE_TIMEOUT_VAR, TOKEN_VAR, WEBSITE_URL};
use crate::db::get_diesel_pool;
use crate::discord::discord_state::DiscordState;
use crate::discord::interactions::{
    button_component, plain_interaction_response, update_resp_to_plain_content,
};
use crate::discord::{notify_racer, REGISTER_CMD, CREATE_SEASON_CMD, CREATE_BRACKET_CMD, SIGNUP_CMD};
use crate::discord::{
    CANCEL_RACE_CMD, CUSTOM_ID_FINISH_RUN, CUSTOM_ID_FORFEIT_MODAL, CUSTOM_ID_FORFEIT_MODAL_INPUT,
    CUSTOM_ID_FORFEIT_RUN, CUSTOM_ID_START_RUN, CUSTOM_ID_USER_TIME, CUSTOM_ID_USER_TIME_MODAL,
    CUSTOM_ID_VOD_MODAL, CUSTOM_ID_VOD_MODAL_INPUT, CUSTOM_ID_VOD_READY,
};
use crate::models::player::{NewPlayer, Player};
use crate::models::race::{NewRace, Race, RaceState};
use crate::models::race_run::RaceRun;
use crate::utils::{env_default, ResultCollapse};
use crate::{Shutdown, Webhooks};
use core::default::Default;
use diesel::result::{DatabaseErrorKind, Error};
use diesel::{Connection, QueryResult, RunQueryDsl, SqliteConnection};
use lazy_static::lazy_static;
use std::fmt::{Debug, Display, Formatter};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use diesel::result::Error::DatabaseError;
use tokio::time::Duration;
use tokio_stream::StreamExt;
use twilight_cache_inmemory::InMemoryCache;
use twilight_gateway::Cluster;
use twilight_http::request::channel::message::UpdateMessage;
use twilight_http::Client;
use twilight_mention::Mention;
use twilight_model::application::command::{BaseCommandOptionData, ChoiceCommandOptionData, CommandOption, CommandOptionType, CommandType, NumberCommandOptionData};
use twilight_model::application::component::button::ButtonStyle;
use twilight_model::application::component::text_input::TextInputStyle;
use twilight_model::application::component::{ActionRow, Component, TextInput};
use twilight_model::application::interaction::application_command::{
    CommandDataOption, CommandOptionValue,
};
use twilight_model::application::interaction::modal::{
    ModalInteractionDataActionRow, ModalSubmitInteraction,
};
use twilight_model::application::interaction::{
    ApplicationCommand, Interaction, MessageComponentInteraction,
};
use twilight_model::gateway::event::Event;
use twilight_model::gateway::payload::incoming::{GuildCreate, InteractionCreate};
use twilight_model::gateway::Intents;
use twilight_model::guild::Permissions;
use twilight_model::http::interaction::{
    InteractionResponse, InteractionResponseData, InteractionResponseType,
};
use twilight_model::id::marker::{ApplicationMarker, ChannelMarker, MessageMarker, UserMarker};
use twilight_model::id::Id;
use twilight_standby::Standby;
use twilight_util::builder::command::CommandBuilder;
use twilight_util::builder::InteractionResponseDataBuilder;
use twilight_validate::message::MessageValidationError;
use crate::models::season::{NewSeason, Season};
use crate::models::signups::NewSignup;

use super::CREATE_RACE_CMD;


macro_rules! get_option_value {
    ($t:ident, $e:expr) => {
        if let CommandOptionValue::$t(output) = $e {
            output
        } else {
            return Err("Invalid option value".to_string());
        }
    };
}

/**
get_opt!("name", &mut vec_of_options, OptionType)

this does something like: find the option with name "name" in the vector,
double check that it has CommandOptionType::OptionType, and then rip the outsides off of the
CommandOptionValue::OptionType(actual_value) and give you back just the actual_value

returns Result<T, String> where actual_value: T
 */
macro_rules! get_opt {
    ($opt_name:expr, $options:expr, $t:ident) => {
        {
            get_opt($opt_name, $options, CommandOptionType::$t)
                .and_then(|opt| {
                    if let CommandOptionValue::$t(output) = opt.value {
                        Ok(output)
                    } else {
                        Err(format!("Invalid option value for {}", $opt_name))
                    }
                }
            )

        }
    }
}

pub(crate) async fn launch(
    webhooks: Webhooks,
    shutdown: tokio::sync::broadcast::Receiver<Shutdown>,
) -> Arc<DiscordState> {
    let token = std::env::var(TOKEN_VAR).unwrap();
    let aid_str = std::env::var(APPLICATION_ID_VAR).unwrap();
    let aid = Id::<ApplicationMarker>::new(aid_str.parse::<u64>().unwrap());

    let http = Client::new(token.clone());
    let cache = InMemoryCache::builder().build();
    let standby = Arc::new(Standby::new());
    let diesel_pool = get_diesel_pool().await;
    let state = Arc::new(DiscordState::new(
        cache,
        http,
        aid,
        diesel_pool,
        webhooks,
        standby.clone(),
    ));

    tokio::spawn(run_bot(token, state.clone(), shutdown));
    state
}

async fn run_bot(
    token: String,
    state: Arc<DiscordState>,
    mut shutdown: tokio::sync::broadcast::Receiver<Shutdown>,
) {
    let intents = Intents::GUILDS
        | Intents::GUILD_MESSAGES
        | Intents::GUILD_MESSAGE_REACTIONS
        | Intents::DIRECT_MESSAGES
        | Intents::DIRECT_MESSAGE_REACTIONS
        | Intents::MESSAGE_CONTENT
        | Intents::GUILD_MEMBERS
        // adding guild presences *solely* to get guild members populated in the cache to avoid
        // subsequent http requests
        | Intents::GUILD_PRESENCES;

    let (cluster, mut events) = Cluster::builder(token.clone(), intents)
        .build()
        .await
        .unwrap();

    let cluster = Arc::new(cluster);
    let cluster_start = cluster.clone();
    let cluster_stop = cluster.clone();
    tokio::spawn(async move {
        cluster_start.up().await;
    });
    loop {
        tokio::select! {
            Some((_shard_id, event)) = events.next() => {
                state.cache.update(&event);
                state.standby.process(&event);
                tokio::spawn(handle_event(event, state.clone()));
            },
            _sd = shutdown.recv() => {
                println!("Twilight bot shutting down...");
                cluster_stop.down();
                break;
            }
        }
    }
    println!("Twilight bot done");
}

/// an ErrorResponse indicates that, rather than simply responding to the interaction with some
/// kind of response, you want to both respond to that (with a plain error message)
/// *AND* inform the admins that there was an error
struct ErrorResponse {
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

trait GetMessageId {
    fn get_message_id(&self) -> Option<Id<MessageMarker>>;
}

impl GetMessageId for MessageComponentInteraction {
    fn get_message_id(&self) -> Option<Id<MessageMarker>> {
        Some(self.message.id)
    }
}

impl GetMessageId for ModalSubmitInteraction {
    fn get_message_id(&self) -> Option<Id<MessageMarker>> {
        self.message.as_ref().map(|m| m.id.clone())
    }
}

async fn handle_create_race(
    mut ac: Box<ApplicationCommand>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    if !state.application_command_run_by_admin(&ac).await? {
        return Err("Unprivileged user attempted to create race".to_string());
    }

    let p1 = get_opt!("p1", &mut ac.data.options, User)?;
    let p2 = get_opt!("p2", &mut ac.data.options, User)?;

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


async fn handle_create_season(
    mut ac: Box<ApplicationCommand>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    if !state.application_command_run_by_admin(&ac).await? {
        return Err("You are not authorized for this.".to_string());
    }
    let format = get_opt!("format", &mut ac.data.options, String)?;

    let ns = NewSeason::new(format);
    let mut cxn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    diesel::insert_into(crate::schema::seasons::table)
        .values(ns)
        .execute(cxn.deref_mut())
        .map_err(|e| e.to_string())?;
    Ok(
        plain_interaction_response("Season created!")
    )
}


async fn handle_create_bracket(
    mut ac: Box<ApplicationCommand>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    if !state.application_command_run_by_admin(&ac).await? {
        return Err("You are not authorized for this.".to_string());
    }
    let name = get_opt!("name", &mut ac.data.options, String)?;
    let season_id = get_opt!("season_id", &mut ac.data.options, Integer)?;
    // TODO: look up season, save thing, whatever

    Err("asdf".to_string())
}

/// turns a "String" error response into a plain interaction response with that text
///
/// designed for use on admin-only commands, where errors should just be reported to the admins
fn admin_command_wrapper(result: Result<Option<InteractionResponse>, String>) -> Result<Option<InteractionResponse>, ErrorResponse> {
    Ok(
        result
        .map_err(|e| Some(plain_interaction_response(e)))
        .collapse()
    )
}

/// extracts the option with given name, if any
/// does not preserve order of remaining opts
/// returns a string representing the error if the expected opt is not found
fn get_opt(
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

const REALLY_CANCEL_ID: &'static str = "really_cancel";

/// returns the new component interaction if the user indicates their choice, otherwise
/// an error indicating what happened instead.
async fn wait_for_cancel_race_decision(
    mid: Id<MessageMarker>,
    state: &Arc<DiscordState>,
) -> Result<MessageComponentInteraction, String> {
    let sb = state.standby.wait_for_component(
        mid,
        // I don't know why but spelling out the parameter type here seems to fix a compiler
        // complaint
        |_: &MessageComponentInteraction| true,
    );

    let time = env_default(CANCEL_RACE_TIMEOUT_VAR, 90);
    match tokio::time::timeout(Duration::from_secs(time), sb).await {
        Ok(cmp) => {
            cmp.map_err(|c| format!("Weird internal error to do with dropping a Standby: {:?}", c))
        }
        Err(_timeout) => {
            Err(format!("This cancellation has timed out, please re-run the command if you still want to cancel."))
        }
    }
}

// this method returns () because it is taking over the interaction flow. we're adding a new
// interaction cycle and not operating on the original interaction anymore.
async fn handle_cancel_race_started(
    ac: Box<ApplicationCommand>,
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
            let resp = if cmp.data.custom_id == REALLY_CANCEL_ID {
                match actually_cancel_race(race, r1, r2, state).await {
                    Ok(()) => "Race cancelled.".to_string(),
                    Err(e) => e,
                }
            } else {
                format!("Okay, not cancelling it.")
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

async fn handle_cancel_race(
    mut ac: Box<ApplicationCommand>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, String> {

    if !state.application_command_run_by_admin(&ac).await? {
        return Err("You are not authorized for this.".to_string());
    }

    let race_id = get_opt!("race_id", &mut ac.data.options, Integer)?;

    if !ac.data.options.is_empty() {
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
        handle_cancel_race_started(ac, race, r1, r2, state)
            .await
            .ok();
        Ok(None)
    } else {
        actually_cancel_race(race, r1, r2, state)
            .await
            .map(|_| Some(plain_interaction_response("Race cancelled.")))
    }
}

// TODO: make this (and handle_application_interaction in general) have error handling
async fn _handle_register(
    mut ac: Box<ApplicationCommand>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, String> {
    let restreams_ok_opt = get_opt!("restream_ok", &mut ac.data.options, Boolean)?;
    let rtgg_username = get_opt!("racetime_username", &mut ac.data.options, String)?;

    let m = ac
        .member
        .ok_or("No member for /register command?".to_string())?;
    let u = m
        .user
        .ok_or(format!("No user for member for /register command"))?;


    let mut cxn = state.diesel_cxn().await.map_err(|e| e.to_string())?;
    let player: Player = cxn.transaction(|conn| {
        use crate::schema::players::dsl::*;
        use diesel::prelude::*;

        if let Ok(Some(p)) = players.filter(discord_id.eq(u.id.to_string()))
            .first(conn)
            .optional() {
            return Ok(p);
        }

        let np = NewPlayer {
            name: u.name,
            discord_id: u.id.to_string(),
            racetime_username: rtgg_username,
            restreams_ok: restreams_ok_opt,
        };

        let res = diesel::insert_into(players)
            .values(np)
            .get_result(conn);
        res
    }).map_err(|e| e.to_string())?;

    let szn = Season::get_active_season(
        cxn.deref_mut()).map_err(|e| e.to_string()
    )?;
    if let Some(season) = szn {
        let ns = NewSignup::new(&player, &season);
        let insert_res = diesel::insert_into(crate::schema::signups::table)
            .values(&ns)
            .execute(cxn.deref_mut());
        if let Err(Error::DatabaseError(DatabaseErrorKind::UniqueViolation, _)) = insert_res {

        }
        match insert_res {
            Ok(_)| Err(Error::DatabaseError(DatabaseErrorKind::UniqueViolation, _)) => {

            },
            Err(e) => {
                return Err(e.to_string());
            }
        }
    }
    Ok(Some(plain_interaction_response("Registered! Thank you :)")))
}


/// this registers the user and signs them up for the current
/// season.
///
/// I am combining these functions because I think it is more user-friendly.
async fn handle_register(
    ac: Box<ApplicationCommand>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    _handle_register(ac, state).await
        .map_err(|e|
            ErrorResponse::new(
                "Sorry, something went wrong with your registration. An admin
                will look into it.",
                e
            )
        )
}

/// this doesn't have an option to return an ErrorResponse because these interactions already occur
/// under the watchful eyes of admins (and are, in fact, run _by_ admins)
async fn handle_application_interaction(
    ac: Box<ApplicationCommand>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    match ac.data.name.as_str() {
        // general commands
        REGISTER_CMD => handle_register(ac, state).await,

        // admin commands
        CREATE_RACE_CMD => admin_command_wrapper(handle_create_race(ac, state).await.map(|i| Some(i))),
        CANCEL_RACE_CMD => admin_command_wrapper(handle_cancel_race(ac, state).await),
        CREATE_SEASON_CMD => admin_command_wrapper(
            handle_create_season(ac, state).await.map(|i| Some(i))
        ),
        CREATE_BRACKET_CMD => admin_command_wrapper(
                handle_create_bracket(ac, state).await.map(|i| Some(i))
        ),

        _ => {
            println!("Unhandled application command: {}", ac.data.name);
            Ok(None)
        }
    }
}

fn run_started_components() -> Vec<Component> {
    action_row(vec![
        button_component("Finish run", CUSTOM_ID_FINISH_RUN, ButtonStyle::Success),
        button_component("Forfeit", CUSTOM_ID_FORFEIT_RUN, ButtonStyle::Danger),
    ])
}

fn action_row(components: Vec<Component>) -> Vec<Component> {
    vec![Component::ActionRow(ActionRow { components })]
}

fn run_started_interaction_response(
    race_run: &RaceRun,
    preamble: Option<&str>,
) -> Result<InteractionResponse, String> {
    let filenames = race_run.filenames()?;
    let content = if let Some(p) = preamble {
        format!(
            "{}

Good luck! your filenames are: `{}`

If anything goes wrong, tell an admin there was an issue with run `{}`
",
            p, filenames, race_run.uuid
        )
    } else {
        format!(
            "Good luck! your filenames are: `{}`

If anything goes wrong, tell an admin there was an issue with run `{}`
",
            filenames, race_run.uuid
        )
    };
    let buttons = run_started_components();
    Ok(InteractionResponse {
        kind: InteractionResponseType::UpdateMessage,
        data: Some(
            InteractionResponseDataBuilder::new()
                .components(buttons)
                .content(content)
                .build(),
        ),
    })
}

async fn handle_run_start(
    interaction: Box<MessageComponentInteraction>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    const USER_FACING_ERROR: &str = "There was an error starting your race. Please ping FoxLisk.";
    let mut conn = state
        .diesel_cxn()
        .await
        .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?;
    let mut rr = RaceRun::get_by_message_id(interaction.message.id, &mut conn)
        .await
        .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?;
    rr.start();
    match rr.save(&mut conn).await {
        Ok(_) => Ok(Some(run_started_interaction_response(&rr, None).map_err(
            |e| {
                ErrorResponse::new(
                    USER_FACING_ERROR,
                    format!("Error sending the /run started/ interaction response {}", e),
                )
            },
        )?)),
        Err(e) => Err(ErrorResponse::new(
            USER_FACING_ERROR,
            format!("Error updating race run: {}", e),
        )),
    }
}

// TODO: this should take a message id, this method signature is bullshit
async fn update_race_run<M, T, F>(
    interaction: &T,
    f: F,
    conn: &mut SqliteConnection,
) -> Result<(), String>
where
    M: GetMessageId,
    T: Deref<Target = M> + Debug,
    F: FnOnce(&mut RaceRun) -> (),
{
    let mid = match interaction.get_message_id() {
        Some(id) => id,
        None => {
            return Err(format!("Interaction {:?} has no message id", interaction));
        }
    };

    let rro = match RaceRun::search_by_message_id(mid.clone(), conn).await {
        Ok(r) => r,
        Err(e) => {
            return Err(e);
        }
    };
    match rro {
        Some(mut rr) => {
            f(&mut rr);
            {
                if let Err(e) = rr.save(conn).await {
                    Err(format!("Error saving race {}: {}", rr.id, e))
                } else {
                    Ok(())
                }
            }
        }
        None => Err(format!("Update for unknown race with message id {}", mid)),
    }
}

fn handle_run_forfeit_button() -> InteractionResponse {
    let ir = create_modal(
        CUSTOM_ID_FORFEIT_MODAL,
        "Forfeit",
        "Enter \"forfeit\" if you want to forfeit",
        vec![Component::TextInput(TextInput {
            custom_id: CUSTOM_ID_FORFEIT_MODAL_INPUT.to_string(),
            label: "Type \"forfeit\" to forfeit.".to_string(),
            max_length: Some(7),
            min_length: Some(7),
            placeholder: None,
            required: Some(true),
            style: TextInputStyle::Short,
            value: None,
        })],
    );
    ir
}

lazy_static! {
    static ref FORFEIT_REGEX: regex::Regex = regex::RegexBuilder::new(r"\s*forfeit\s*")
        .case_insensitive(true)
        .build()
        .unwrap();
}

async fn handle_run_forfeit_modal(
    mut interaction: Box<ModalSubmitInteraction>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    const USER_FACING_ERROR: &str =
        "Something went wrong forfeiting this race. Please ping FoxLisk.";

    let ut = get_field_from_modal_components(
        std::mem::take(&mut interaction.data.components),
        CUSTOM_ID_FORFEIT_MODAL_INPUT,
    )
    .ok_or(ErrorResponse::new(
        USER_FACING_ERROR,
        "Error getting forfeit input from modal.",
    ))?;
    let mut conn = state
        .diesel_cxn()
        .await
        .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?;
    let ir = if FORFEIT_REGEX.is_match(&ut) {
        update_race_run(&interaction, |rr| rr.forfeit(), &mut conn)
            .await
            .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?;

        update_resp_to_plain_content(
            "You have forfeited this match. Please let the admins know if there are any issues.",
        )
    } else {
        let mid = match interaction.get_message_id() {
            Some(id) => id,
            None => {
                return Err(ErrorResponse::new(
                    USER_FACING_ERROR,
                    format!("Interaction {:?} has no message id", interaction),
                ));
            }
        };

        RaceRun::get_by_message_id(mid, &mut conn)
            .await
            .and_then(|race_run| {
                run_started_interaction_response(&race_run, Some("Forfeit canceled"))
            })
            .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?
    };
    Ok(Some(ir))
}

fn create_modal(
    custom_id: &str,
    content: &str,
    title: &str,
    components: Vec<Component>,
) -> InteractionResponse {
    InteractionResponse {
        kind: InteractionResponseType::Modal,
        data: Some(InteractionResponseData {
            components: Some(action_row(components)),
            content: Some(content.to_string()),
            custom_id: Some(custom_id.to_string()),
            title: Some(title.to_string()),
            ..Default::default()
        }),
    }
}

async fn handle_run_finish(
    interaction: Box<MessageComponentInteraction>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    const USER_FACING_ERROR: &str = "Something went wrong finishing this run. Please ping FoxLisk.";
    let mut conn = state
        .diesel_cxn()
        .await
        .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?;
    if let Err(e) = update_race_run(
        &interaction,
        |rr| {
            rr.finish();
        },
        &mut conn,
    )
    .await
    {
        // TODO: this should maybe be updating a response?
        return Err(ErrorResponse::new(
            USER_FACING_ERROR,
            format!("Error persisting finished run: {}", e),
        ));
    }
    let ir = create_modal(
        CUSTOM_ID_USER_TIME_MODAL,
        "Please enter finish time in **H:MM:SS** format",
        "Enter finish time in **H:MM:SS** format",
        vec![Component::TextInput(TextInput {
            custom_id: CUSTOM_ID_USER_TIME.to_string(),
            label: "Finish time:".to_string(),
            max_length: Some(100),
            min_length: Some(5),
            placeholder: None,
            required: Some(true),
            style: TextInputStyle::Short,
            value: None,
        })],
    );
    Ok(Some(ir))
}

fn get_field_from_modal_components(
    rows: Vec<ModalInteractionDataActionRow>,
    custom_id: &str,
) -> Option<String> {
    for row in rows {
        for cmp in row.components {
            // modal interaction components can be ActionRows, but they can't have sub-components?
            // I don't really get what's going on here, but I think a modal is basically just
            // an action row + some text inputs. that's all I'm doing, anyway, so it's fine
            if cmp.custom_id == custom_id {
                return Some(cmp.value);
            }
        }
    }
    None
}

async fn handle_user_time_modal(
    mut interaction: Box<ModalSubmitInteraction>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    const USER_FACING_ERROR: &str =
        "Something went wrong reporting your time. Please ping FoxLisk.";
    let ut = get_field_from_modal_components(
        std::mem::take(&mut interaction.data.components),
        CUSTOM_ID_USER_TIME,
    )
    .ok_or(ErrorResponse::new(
        USER_FACING_ERROR,
        "Error getting user time form modal.",
    ))?;
    let mut conn = state
        .diesel_cxn()
        .await
        .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?;
    update_race_run(&interaction, |rr| rr.report_user_time(ut), &mut conn)
        .await
        .map_err(|e| {
            ErrorResponse::new(
                "Something went wrong reporting your time. Please ping FoxLisk.",
                e,
            )
        })?;

    let ir = InteractionResponse {
        kind: InteractionResponseType::UpdateMessage,
        data: Some(InteractionResponseData {
            components: Some(action_row(vec![button_component(
                "VoD ready",
                CUSTOM_ID_VOD_READY,
                ButtonStyle::Success,
            )])),
            content: Some("Please click here once your VoD is ready".to_string()),
            ..Default::default()
        }),
    };
    Ok(Some(ir))
}

fn handle_vod_ready() -> InteractionResponse {
    let ir = create_modal(
        CUSTOM_ID_VOD_MODAL,
        "Please enter your VoD URL",
        "VoD URL",
        vec![Component::TextInput(TextInput {
            custom_id: CUSTOM_ID_VOD_MODAL_INPUT.to_string(),
            label: "Enter VoD here".to_string(),
            max_length: None,
            min_length: Some(5),
            placeholder: None,
            required: Some(true),
            style: TextInputStyle::Short,
            value: None,
        })],
    );
    ir
}

async fn handle_vod_modal(
    mut interaction: Box<ModalSubmitInteraction>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    const USER_FACING_ERROR: &str = "Something went wrong reporting your VoD. Please ping FoxLisk.";
    let user_input = get_field_from_modal_components(
        std::mem::take(&mut interaction.data.components),
        CUSTOM_ID_VOD_MODAL_INPUT,
    )
    .ok_or(ErrorResponse::new(
        USER_FACING_ERROR,
        "Error getting vod from modal.",
    ))?;
    let mut conn = state
        .diesel_cxn()
        .await
        .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?;

    update_race_run(&interaction, |rr| rr.set_vod(user_input), &mut conn)
        .await
        .map_err(|e| {
            ErrorResponse::new(
                "Something went wrong reporting your VoD. Please ping FoxLisk.",
                format!("Error saving vod reporting: {}", e),
            )
        })?;
    let ir = plain_interaction_response(
        "Thank you, your race is completed. Please message the admins if there are any issues.",
    );
    Ok(Some(ir))
}

async fn handle_button_interaction(
    interaction: Box<MessageComponentInteraction>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    match interaction.data.custom_id.as_str() {
        CUSTOM_ID_START_RUN => handle_run_start(interaction, state).await,
        CUSTOM_ID_FORFEIT_RUN => Ok(Some(handle_run_forfeit_button())),
        CUSTOM_ID_FINISH_RUN => handle_run_finish(interaction, state).await,
        CUSTOM_ID_VOD_READY => Ok(Some(handle_vod_ready())),
        _ => {
            println!("Unhandled button: {:?}", interaction);
            Ok(None)
        }
    }
}

async fn handle_modal_submission(
    interaction: Box<ModalSubmitInteraction>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    match interaction.data.custom_id.as_str() {
        CUSTOM_ID_USER_TIME_MODAL => handle_user_time_modal(interaction, state).await,
        CUSTOM_ID_VOD_MODAL => handle_vod_modal(interaction, state).await,
        CUSTOM_ID_FORFEIT_MODAL => handle_run_forfeit_modal(interaction, state).await,
        _ => {
            println!("Unhandled modal: {:?}", interaction);
            Ok(None)
        }
    }
}

async fn _handle_interaction(
    interaction: InteractionCreate,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    match interaction.0 {
        Interaction::ApplicationCommand(ac) => handle_application_interaction(ac, &state).await,
        Interaction::MessageComponent(mc) => handle_button_interaction(mc, &state).await,
        Interaction::ModalSubmit(ms) => handle_modal_submission(ms, &state).await,
        _ => {
            println!("Unhandled interaction: {:?}", interaction);
            Ok(None)
        }
    }
}

/// Handles an interaction. This attempts to dispatch to the relevant processing code, and then
/// creates any responses as specified, and alerts admins via webhook if there is a problem.
async fn handle_interaction(interaction: InteractionCreate, state: Arc<DiscordState>) {
    let interaction_id = interaction.id();
    let token = interaction.token().to_string();

    let (user_resp, admin_message) = match _handle_interaction(interaction, &state).await {
        Ok(o) => (o, None),
        Err(e) => (
            Some(plain_interaction_response(e.user_facing_error)),
            Some(e.internal_error),
        ),
    };

    let mut final_message = String::new();
    if let Some(m) = admin_message {
        final_message.push_str(&format!("Encountered an error: {}", m));
    }

    if let Some(u) = user_resp {
        if let Some(more_errors) = state
            .create_response_err_to_str(interaction_id, &token, &u)
            .await
            .err()
        {
            final_message.push_str(&format!(
                "Unable to communicate error to user: {}",
                more_errors
            ));
        }
    }

    if !final_message.is_empty() {
        println!("{}", final_message);
        if let Err(e) = state.webhooks.message_async(&final_message).await {
            // at some point you just have to give up
            println!("Error reporting internal error: {}", e);
        }
    }
}

async fn set_application_commands(
    gc: &Box<GuildCreate>,
    state: Arc<DiscordState>,
) -> Result<(), String> {
    let create_race = CommandBuilder::new(
        CREATE_RACE_CMD.to_string(),
        "Create an asynchronous race for two players".to_string(),
        CommandType::ChatInput,
    )
    .default_member_permissions(Permissions::MANAGE_GUILD)
    .option(CommandOption::User(BaseCommandOptionData {
        description: "First racer".to_string(),
        description_localizations: None,
        name: "p1".to_string(),
        name_localizations: None,
        required: true,
    }))
    .option(CommandOption::User(BaseCommandOptionData {
        description: "Second racer".to_string(),
        description_localizations: None,
        name: "p2".to_string(),
        name_localizations: None,
        required: true,
    }))
    .build();

    let cancel_race = CommandBuilder::new(
        CANCEL_RACE_CMD.to_string(),
        "Cancel an existing asynchronous race".to_string(),
        CommandType::ChatInput,
    )
    .default_member_permissions(Permissions::MANAGE_GUILD)
    .option(CommandOption::Integer(NumberCommandOptionData {
        autocomplete: false,
        choices: vec![],
        description: format!("Race ID. Get this from {}", WEBSITE_URL),
        description_localizations: None,
        max_value: None,
        min_value: None,
        name: "race_id".to_string(),
        name_localizations: None,
        required: true,
    }))
    .build();

    let register = CommandBuilder::new(
        REGISTER_CMD.to_string(),
        "Register with League and signup for the current season."
            .to_string(),
        CommandType::ChatInput,
    )
    .option(CommandOption::Boolean(BaseCommandOptionData {
        description: "Are you okay with being restreamed?".to_string(),
        description_localizations: None,
        name: "restream_ok".to_string(),
        name_localizations: None,
        required: true,
    }))
        .option(CommandOption::String(
            ChoiceCommandOptionData {
                autocomplete: false,
                choices: vec![],
                description: "RaceTime.gg account with discriminator: username#1234".to_string(),
                description_localizations: None,
                name: "racetime_username".to_string(),
                name_localizations: None,
                required: true
            }
        ))
    .build();


    let create_season =  CommandBuilder::new(
        CREATE_SEASON_CMD.to_string(),
        "Create a new season"
            .to_string(),
        CommandType::ChatInput,
    )
        .option(CommandOption::String(
            ChoiceCommandOptionData {
                autocomplete: false,
                choices: vec![],
                description: "Format (e.g. Any% NMG)".to_string(),
                description_localizations: None,
                name: "format".to_string(),
                name_localizations: None,
                required: true
            }
        ))
        .build();


    let create_bracket =  CommandBuilder::new(
        CREATE_BRACKET_CMD.to_string(),
        "Create a new bracket"
            .to_string(),
        CommandType::ChatInput,
    )
        .option(CommandOption::String(
            ChoiceCommandOptionData {
                autocomplete: false,
                choices: vec![],
                description: "Name (e.g. Dark World)".to_string(),
                description_localizations: None,
                name: "name".to_string(),
                name_localizations: None,
                required: true
            }
        ))
        .option(CommandOption::Integer(
            NumberCommandOptionData {
                autocomplete: false,
                choices: vec![],
                description: "Season ID".to_string(),
                description_localizations: None,
                max_value: None,
                min_value: None,
                name: "season_id".to_string(),
                name_localizations: None,
                required: true
            }
        ))
        .build();

    let commands = vec![
        create_race, cancel_race, register, create_season,
        create_bracket,

    ];

    let resp = state
        .interaction_client()
        .set_guild_commands(gc.id.clone(), &commands)
        .exec()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!(
            "Error response setting guild commands: {}",
            resp.status()
        ));
    }
    Ok(())
}

async fn handle_event(event: Event, state: Arc<DiscordState>) {
    match event {
        Event::GuildCreate(gc) => {
            if let Err(e) = set_application_commands(&gc, state).await {
                println!(
                    "Error setting application commands for guild {:?}: {}",
                    gc.id, e
                );
            }
        }
        Event::GuildDelete(gd) => {
            println!("Guild deleted?? event: {:?}", gd);
        }

        Event::GuildUpdate(gu) => {
            println!("Guild updated event: {:?}", gu);
        }
        Event::InteractionCreate(ic) => {
            handle_interaction(ic, state).await;
        }
        Event::Ready(r) => {
            println!("Ready! {:?}", r);
        }
        Event::RoleDelete(rd) => {
            println!("Role deleted: {:?}", rd);
        }
        Event::RoleUpdate(ru) => {
            println!("Role updated: {:?}", ru);
        }
        _ => {}
    }
}
