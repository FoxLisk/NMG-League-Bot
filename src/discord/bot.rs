use core::default::Default;
use std::sync::Arc;

use diesel::SqliteConnection;
use lazy_static::lazy_static;
use log::{debug, info, warn};
use nmg_league_bot::config::CONFIG;
use racetime_api::client::RacetimeClient;
use tokio_stream::StreamExt;
use twilight_cache_inmemory::InMemoryCache;
use twilight_gateway::stream::ShardEventStream;
use twilight_gateway::{stream, Config};
use twilight_http::Client;
use twilight_model::application::interaction::message_component::MessageComponentInteractionData;
use twilight_model::application::interaction::modal::{
    ModalInteractionData, ModalInteractionDataActionRow,
};
use twilight_model::application::interaction::InteractionData;
use twilight_model::channel::message::component::{ButtonStyle, TextInput, TextInputStyle};
use twilight_model::channel::message::Component;
use twilight_model::gateway::event::Event;
use twilight_model::gateway::payload::incoming::{GuildCreate, InteractionCreate};
use twilight_model::gateway::Intents;
use twilight_model::http::interaction::{
    InteractionResponse, InteractionResponseData, InteractionResponseType,
};
use twilight_model::id::marker::MessageMarker;
use twilight_model::id::Id;
use twilight_standby::Standby;
use twilight_util::builder::InteractionResponseDataBuilder;

use crate::discord::application_command_definitions::application_command_definitions;
use crate::discord::components::action_row;
use crate::discord::constants::{
    CUSTOM_ID_FINISH_RUN, CUSTOM_ID_FORFEIT_MODAL, CUSTOM_ID_FORFEIT_MODAL_INPUT,
    CUSTOM_ID_FORFEIT_RUN, CUSTOM_ID_START_RUN, CUSTOM_ID_USER_TIME, CUSTOM_ID_USER_TIME_MODAL,
    CUSTOM_ID_VOD_MODAL, CUSTOM_ID_VOD_MODAL_INPUT, CUSTOM_ID_VOD_READY,
};
use crate::discord::discord_state::DiscordState;
use crate::discord::interaction_handlers::application_commands::handle_application_interaction;
use crate::discord::interactions_utils::{
    button_component, plain_interaction_response, update_resp_to_plain_content,
};
use crate::discord::reaction_handlers::{handle_reaction_add, handle_reaction_remove};
use crate::discord::ErrorResponse;
use crate::{Shutdown, Webhooks};
use nmg_league_bot::db::get_diesel_pool;
use nmg_league_bot::models::asyncs::race::AsyncRace;
use nmg_league_bot::models::asyncs::race_run::AsyncRaceRun;
use nmg_league_bot::twitch_client::TwitchClientBundle;
use nmg_league_bot::utils::ResultErrToString;

pub(crate) async fn launch(
    webhooks: Webhooks,
    racetime_client: RacetimeClient,
    twitch_client_bundle: TwitchClientBundle,
    shutdown: tokio::sync::broadcast::Receiver<Shutdown>,
) -> Arc<DiscordState> {
    let aid = CONFIG.discord_application_id;

    let http = Client::new(CONFIG.discord_token.clone());
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
        racetime_client,
        twitch_client_bundle,
    ));

    tokio::spawn(run_bot(
        CONFIG.discord_token.clone(),
        state.clone(),
        shutdown,
    ));
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

    let cfg = Config::builder(token.clone(), intents).build();

    let mut shards = stream::create_recommended(&state.client, cfg, |_, builder| builder.build())
        .await
        .unwrap()
        .collect::<Vec<_>>();

    // N.B. collecting these into a vec and then using `.iter_mut()` is stupid, but idk how to
    // convince the compiler that i have an iterator of mutable references in a simpler way
    let mut events = ShardEventStream::new(shards.iter_mut());

    loop {
        tokio::select! {
            Some((_shard_id, evt)) = events.next() => {
                match evt {
                    Ok(event) => {
                        state.cache.update(&event);
                        state.standby.process(&event);
                        tokio::spawn(handle_event(event, state.clone()));
                    }
                    Err(e) => {
                        warn!("Got error receiving discord event: {e}");
                        if e.is_fatal() {
                            info!("Twilight bot shutting down due to fatal error");
                            break;
                        }
                    }
                }
            },

            _sd = shutdown.recv() => {
                info!("Twilight bot shutting down...");
                break;
            }
        }
    }
    info!("Twilight bot done");
}

fn interaction_to_message_id<S: Into<String>>(
    i: &InteractionCreate,
    user_facing_err: S,
) -> Result<Id<MessageMarker>, ErrorResponse> {
    i.message
        .as_ref()
        .map(|m| m.id)
        .ok_or(ErrorResponse::new(user_facing_err, "Missing message id?"))
}

fn run_started_components() -> Vec<Component> {
    action_row(vec![
        button_component("Finish run", CUSTOM_ID_FINISH_RUN, ButtonStyle::Success),
        button_component("Forfeit", CUSTOM_ID_FORFEIT_RUN, ButtonStyle::Danger),
    ])
}

fn run_started_interaction_response(
    race: &AsyncRace,
    race_run: &AsyncRaceRun,
    preamble: Option<&str>,
) -> Result<InteractionResponse, String> {
    let filenames = race_run.filenames()?;
    let admin_text = if let Some(msg_text) = race.on_start_message.as_ref() {
        format!("\nThe admin who created this race had the following information for you

> {msg_text}\n")
    } else {
        "".to_string()
    };
    let preamble_content = if let Some(preamble_text) = preamble {
        format!("{preamble_text}\n\n")
    } else {
        "".to_string()
    };
    let content = format!("\
{preamble_content}Good luck! your filenames are: `{filenames}`
{admin_text}
If anything goes wrong, tell an admin there was an issue with run `{}`
",
           race_run.uuid
        );
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
    _component_data: MessageComponentInteractionData,
    interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    const USER_FACING_ERROR: &str = "There was an error starting your race. Please ping FoxLisk.";
    let mid = interaction
        .message
        .as_ref()
        .map(|m| m.id)
        .ok_or(ErrorResponse::new(USER_FACING_ERROR, "Missing message id?"))?;
    let mut conn = state
        .diesel_cxn()
        .await
        .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?;
    let mut rr = AsyncRaceRun::get_by_message_id(mid, &mut conn)
        .await
        .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?;
    let race = rr.get_race(&mut conn).map_err(
        |e| ErrorResponse::new(USER_FACING_ERROR, e.to_string())
    )?;
    rr.start();
    match rr.save(&mut conn).await {
        Ok(_) => Ok(Some(run_started_interaction_response(&race, &rr, None).map_err(
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

async fn update_race_run<F>(
    message_id: Id<MessageMarker>,
    f: F,
    conn: &mut SqliteConnection,
) -> Result<(), String>
where
    F: FnOnce(&mut AsyncRaceRun) -> (),
{
    let rro = match AsyncRaceRun::search_by_message_id(message_id.clone(), conn).await {
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
        None => Err(format!(
            "Update for unknown race with message id {}",
            message_id
        )),
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

// this is the method that handles the last forfeit step, after the player enters 'forfeit' and submits that
async fn handle_run_forfeit_modal(
    mut interaction_data: ModalInteractionData,
    interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    const USER_FACING_ERROR: &str =
        "Something went wrong forfeiting this race. Please ping FoxLisk.";
    let mid = interaction_to_message_id(&interaction, USER_FACING_ERROR)?;
    let ut = get_field_from_modal_components(
        std::mem::take(&mut interaction_data.components),
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
        update_race_run(mid, |rr| rr.forfeit(), &mut conn)
            .await
            .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?;

        update_resp_to_plain_content(
            "You have forfeited this match. Please let the admins know if there are any issues.",
        )
    } else {
        AsyncRaceRun::get_by_message_id(mid, &mut conn)
            .await
            .and_then(|race_run|
                race_run.get_race(&mut conn)
                    .map(|race| (race, race_run))
                    .map_err_to_string()
            )
            .and_then(
                |(race, race_run)|
                    run_started_interaction_response(&race, &race_run, Some("Forfeit canceled"))
            )
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
    interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    const USER_FACING_ERROR: &str = "Something went wrong finishing this run. Please ping FoxLisk.";
    let mid = interaction_to_message_id(&interaction, USER_FACING_ERROR)?;
    let mut conn = state
        .diesel_cxn()
        .await
        .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?;
    if let Err(e) = update_race_run(
        mid,
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
                return cmp.value;
            }
        }
    }
    None
}

async fn handle_user_time_modal(
    mut interaction_data: ModalInteractionData,
    interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    const USER_FACING_ERROR: &str =
        "Something went wrong reporting your time. Please ping FoxLisk.";
    let mid = interaction_to_message_id(&interaction, USER_FACING_ERROR)?;
    let ut = get_field_from_modal_components(
        std::mem::take(&mut interaction_data.components),
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
    update_race_run(mid, |rr| rr.report_user_time(ut), &mut conn)
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
    mut interaction_data: ModalInteractionData,
    interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    const USER_FACING_ERROR: &str = "Something went wrong reporting your VoD. Please ping FoxLisk.";
    let mid = interaction_to_message_id(&interaction, USER_FACING_ERROR)?;
    let user_input = get_field_from_modal_components(
        std::mem::take(&mut interaction_data.components),
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

    update_race_run(mid, |rr| rr.set_vod(user_input), &mut conn)
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
    interaction_data: MessageComponentInteractionData,
    interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    match interaction_data.custom_id.as_str() {
        CUSTOM_ID_START_RUN => handle_run_start(interaction_data, interaction, state).await,
        CUSTOM_ID_FORFEIT_RUN => Ok(Some(handle_run_forfeit_button())),
        CUSTOM_ID_FINISH_RUN => handle_run_finish(interaction, state).await,
        CUSTOM_ID_VOD_READY => Ok(Some(handle_vod_ready())),
        _ => {
            info!("Unhandled button: {:?}", interaction_data);
            Ok(None)
        }
    }
}

async fn handle_modal_submission(
    interaction_data: ModalInteractionData,
    interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    match interaction_data.custom_id.as_str() {
        CUSTOM_ID_USER_TIME_MODAL => {
            handle_user_time_modal(interaction_data, interaction, state).await
        }
        CUSTOM_ID_VOD_MODAL => handle_vod_modal(interaction_data, interaction, state).await,
        CUSTOM_ID_FORFEIT_MODAL => {
            handle_run_forfeit_modal(interaction_data, interaction, state).await
        }
        _ => {
            info!("Unhandled modal: {:?}", interaction_data);
            Ok(None)
        }
    }
}

async fn _handle_interaction(
    mut interaction: Box<InteractionCreate>,
    state: &Arc<DiscordState>,
) -> Result<Option<InteractionResponse>, ErrorResponse> {
    let data = std::mem::take(&mut interaction.0.data);
    if let Some(id) = data {
        match id {
            InteractionData::ApplicationCommand(ac) => {
                handle_application_interaction(ac, interaction, &state).await
            }
            InteractionData::MessageComponent(mc) => {
                handle_button_interaction(mc, interaction, &state).await
            }
            InteractionData::ModalSubmit(ms) => {
                handle_modal_submission(ms, interaction, &state).await
            }
            _ => {
                warn!("Unhandled interaction: {:?}", interaction);
                Ok(None)
            }
        }
    } else {
        // this handles the PING case, which is webhook-only, aiui
        Ok(None)
    }
}

/// Handles an interaction. This attempts to dispatch to the relevant processing code, and then
/// creates any responses as specified, and alerts admins via webhook if there is a problem.
async fn handle_interaction(interaction: Box<InteractionCreate>, state: Arc<DiscordState>) {
    let interaction_id = interaction.id;
    let token = interaction.token.clone();

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
        info!("handle_interaction trying to send response {:?}", u);
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
        warn!("{}", final_message);
        if let Err(e) = state.webhooks.message_async(&final_message).await {
            // at some point you just have to give up
            warn!("Error reporting internal error: {}", e);
        }
    }
}

async fn set_application_commands(
    gc: &Box<GuildCreate>,
    state: Arc<DiscordState>,
) -> Result<(), String> {
    let commands = application_command_definitions();
    let resp = state
        .interaction_client()
        .set_guild_commands(gc.id.clone(), &commands)
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
                warn!(
                    "Error setting application commands for guild {:?}: {}",
                    gc.id, e
                );
            }
        }
        Event::GuildDelete(gd) => {
            debug!("Guild deleted?? event: {:?}", gd);
        }

        Event::GuildUpdate(gu) => {
            debug!("Guild updated event: {:?}", gu);
        }
        Event::InteractionCreate(ic) => {
            handle_interaction(ic, state).await;
        }
        Event::Ready(r) => {
            info!("Ready! {:?}", r);
        }
        Event::RoleDelete(rd) => {
            debug!("Role deleted: {:?}", rd);
        }
        Event::RoleUpdate(ru) => {
            debug!("Role updated: {:?}", ru);
        }
        Event::ReactionAdd(ra) => {
            handle_reaction_add(ra, &state).await;
        }
        Event::ReactionRemove(rr) => {
            handle_reaction_remove(rr, &state).await;
        }
        _ => {}
    }
}
