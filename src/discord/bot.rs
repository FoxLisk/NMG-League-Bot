use core::default::Default;
use std::sync::Arc;

use diesel::{ SqliteConnection};
use lazy_static::lazy_static;
use tokio_stream::StreamExt;
use twilight_cache_inmemory::InMemoryCache;
use twilight_gateway::Cluster;
use twilight_http::Client;
use twilight_model::application::component::button::ButtonStyle;
use twilight_model::application::component::text_input::TextInputStyle;
use twilight_model::application::component::{Component, TextInput};
use twilight_model::application::interaction::message_component::MessageComponentInteractionData;
use twilight_model::application::interaction::modal::{
    ModalInteractionData, ModalInteractionDataActionRow,
};
use twilight_model::application::interaction::{ InteractionData};
use twilight_model::gateway::event::Event;
use twilight_model::gateway::payload::incoming::{GuildCreate, InteractionCreate};
use twilight_model::gateway::Intents;
use twilight_model::http::interaction::{
    InteractionResponse, InteractionResponseData, InteractionResponseType,
};
use twilight_model::id::marker::{ApplicationMarker, MessageMarker};
use twilight_model::id::Id;
use twilight_standby::Standby;
use twilight_util::builder::InteractionResponseDataBuilder;

use crate::constants::{APPLICATION_ID_VAR, TOKEN_VAR};
use crate::db::get_diesel_pool;
use crate::discord::discord_state::DiscordState;
use crate::discord::interactions_utils::{
    button_component, plain_interaction_response,
    update_resp_to_plain_content,
};
use crate::discord::{ErrorResponse};
use crate::discord::{
     CUSTOM_ID_FINISH_RUN, CUSTOM_ID_FORFEIT_MODAL, CUSTOM_ID_FORFEIT_MODAL_INPUT,
    CUSTOM_ID_FORFEIT_RUN, CUSTOM_ID_START_RUN, CUSTOM_ID_USER_TIME, CUSTOM_ID_USER_TIME_MODAL,
    CUSTOM_ID_VOD_MODAL, CUSTOM_ID_VOD_MODAL_INPUT, CUSTOM_ID_VOD_READY,
};
use nmg_league_bot::models::race_run::RaceRun;
use nmg_league_bot::utils::env_var;
use crate::{Shutdown, Webhooks};
use crate::discord::application_commands::application_command_definitions;
use crate::discord::components::action_row;
use crate::discord::interaction_handlers::application_commands::handle_application_interaction;
use crate::discord::reaction_handlers::{handle_reaction_add, handle_reaction_remove};


pub(crate) async fn launch(
    webhooks: Webhooks,
    shutdown: tokio::sync::broadcast::Receiver<Shutdown>,
) -> Arc<DiscordState> {
    let token =  env_var(TOKEN_VAR);
    let aid_str = env_var(APPLICATION_ID_VAR);
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
    let mut rr = RaceRun::get_by_message_id(mid, &mut conn)
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
async fn update_race_run<F>(
    message_id: Id<MessageMarker>,
    f: F,
    conn: &mut SqliteConnection,
) -> Result<(), String>
where
    F: FnOnce(&mut RaceRun) -> (),
{
    let rro = match RaceRun::search_by_message_id(message_id.clone(), conn).await {
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
            println!("Unhandled button: {:?}", interaction_data);
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
            println!("Unhandled modal: {:?}", interaction_data);
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
                println!("Unhandled interaction: {:?}", interaction);
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
        println!("handle_interaction trying to send response {:?}", u);
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

    let commands = application_command_definitions();
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
        Event::ReactionAdd(ra) => {
            handle_reaction_add(ra, &state).await;
        }
        Event::ReactionRemove(rr) => {
            handle_reaction_remove(rr, &state).await;
        }
        _ => {}
    }
}
