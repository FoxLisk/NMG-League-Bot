use crate::constants::{APPLICATION_ID_VAR, TOKEN_VAR, WEBSITE_URL};
use crate::db::get_pool;
use crate::discord::discord_state::DiscordState;
use crate::discord::{CANCEL_RACE_CMD, CUSTOM_ID_FINISH_RUN, CUSTOM_ID_FORFEIT_MODAL, CUSTOM_ID_FORFEIT_MODAL_INPUT, CUSTOM_ID_FORFEIT_RUN, CUSTOM_ID_START_RUN, CUSTOM_ID_USER_TIME, CUSTOM_ID_USER_TIME_MODAL, CUSTOM_ID_VOD_MODAL, CUSTOM_ID_VOD_MODAL_INPUT, CUSTOM_ID_VOD_READY, interactions};
use crate::models::race::{NewRace, Race};
use crate::models::race_run::RaceRun;
use crate::{Shutdown, Webhooks};
use core::default::Default;
use std::fmt::{Debug, Display};
use std::ops::Deref;
use std::sync::Arc;
use tokio_stream::StreamExt;
use twilight_cache_inmemory::InMemoryCache;
use twilight_gateway::Cluster;
use twilight_http::Client;
use twilight_mention::Mention;
use twilight_model::application::command::{BaseCommandOptionData, CommandOption, CommandType, NumberCommandOptionData};
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
use twilight_model::id::marker::{ApplicationMarker, MessageMarker, UserMarker};
use twilight_model::id::Id;
use twilight_util::builder::command::CommandBuilder;
use twilight_util::builder::InteractionResponseDataBuilder;
use lazy_static::lazy_static;
use twilight_http::request::application::command::UpdateCommandPermissions;
use twilight_http::response::DeserializeBodyError;
use twilight_model::application::command::permissions::{CommandPermissions, CommandPermissionsType};
use twilight_validate::command::CommandValidationError;
use crate::discord::interactions::{plain_interaction_response, update_resp_to_plain_content};
use crate::discord::notify_racer;

use super::CREATE_RACE_CMD;

pub(crate) async fn launch(
    webhooks: Webhooks,
    shutdown: tokio::sync::broadcast::Receiver<Shutdown>,
) -> Arc<DiscordState> {
    let token = std::env::var(TOKEN_VAR).unwrap();
    let aid_str = std::env::var(APPLICATION_ID_VAR).unwrap();
    let aid = Id::<ApplicationMarker>::new(aid_str.parse::<u64>().unwrap());
    let pool = get_pool().await.unwrap();

    let http = Client::new(token.clone());
    let cache = InMemoryCache::builder().build();
    let state = Arc::new(DiscordState::new(cache, http, aid, pool, webhooks));

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


struct ErrorResponse {
    user_facing_error: String,
    internal_error: Option<String>,
}

impl ErrorResponse {
    fn new<S1: Into<String>, S2: Display>(user_facing_error: S1, internal_error: S2) -> Self {
        Self {
            user_facing_error: user_facing_error.into(),
            internal_error: Some(internal_error.to_string()),
        }
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

// this is doing all the work but it's being wrapped just to make refactoring easier
async fn _handle_create_race(
    mut ac: Box<ApplicationCommand>,
    state: &Arc<DiscordState>,
) -> Result<InteractionResponse, String> {
    let gid = ac
        .guild_id
        .ok_or("Create race called outside of guild context".to_string())?;
    let uid = ac
        .author_id()
        .ok_or("Create race called by no one ????".to_string())?;

    if !state.has_admin_role(uid, gid).await? {
        return Err("Unprivileged user attempted to create race".to_string());
    }

    if ac.data.options.len() != 2 {
        return Err(format!(
            "Expected exactly 2 options for {}",
            CREATE_RACE_CMD
        ));
    }

    fn get_user(cdos: &mut Vec<CommandDataOption>) -> Option<Id<UserMarker>> {
        let opt = cdos.pop()?;
        match opt.value {
            CommandOptionValue::User(uid) => Some(uid),
            _ => None,
        }
    }

    let p1 = get_user(&mut ac.data.options).ok_or("Expected two users".to_string())?;
    let p2 = get_user(&mut ac.data.options).ok_or("Expected two users".to_string())?;
    if p1 == p2 {
        return Ok(interactions::plain_interaction_response(
            "The racers must be different users",
        ));
    }

    let new_race = NewRace::new();
    let race = new_race
        .save(&state.pool)
        .await
        .map_err(|e| format!("Error saving race: {}", e))?;
    let (mut r1, mut r2) = race
        .select_racers(p1.clone(), p2.clone(), &state.pool)
        .await
        .map_err(|e| format!("Error saving race: {}", e))?;

    let (first, second) = {
        tokio::join!(
            notify_racer(&mut r1, &race, &state),
            notify_racer(&mut r2, &race, &state)
        )
    };
    // this is annoying, i havent really found a pattern i like for "report 0-2 errors" in Rust yet
    match (first, second) {
        (Ok(_), Ok(_)) => {
            Ok(plain_interaction_response(format!(
                "Race created for users {} and {}",
                p1.mention(),
                p2.mention(),
            )))
        },
        (Err(e), Ok(_)) => {
            Ok(
                plain_interaction_response(format!("Error creating race: error contacting {}: {}",
                    p1.mention(),
                    e
                ))
            )
        },
        (Ok(_), Err(e)) => {
            Ok(plain_interaction_response(format!("Error creating race: error contacting {}: {}",
                                               p2.mention(),
                                               e
            )))
        },
        (Err(e1), Err(e2)) => {
            Ok(plain_interaction_response(format!("Error creating race: error contacting {}: {} \
            error contacting {}: {}",
                                               p1.mention(),
                                               e1,
                p2.mention(),
                e2
            )))
        },
    }
}


async fn handle_create_race(
    mut ac: Box<ApplicationCommand>,
    state: &Arc<DiscordState>,
) -> Result<(), ErrorResponse> {
    let interaction_id = ac.id;
    let token = ac.token.to_string();
    match _handle_create_race(ac, &state).await {
        Ok(r) => state
            .interaction_client()
            .create_response(interaction_id, &token, &r)
            .exec()
            .await
            .map(|_| ())
            .map_err(|e| ErrorResponse {
                user_facing_error: format!("Error creating race: {}", e),
                internal_error: None,
            }),
        Err(e) => Err(ErrorResponse {
            user_facing_error: format!("Internal error creating race: {}", e),
            internal_error: None,
        }),
    }
}

async fn handle_application_interaction(
    mut ac: Box<ApplicationCommand>,
    state: &Arc<DiscordState>,
) -> Result<(), ErrorResponse> {
    match ac.data.name.as_str() {
        CREATE_RACE_CMD => handle_create_race(ac, state).await,
        _ => {
            println!("Unhandled application command: {}", ac.data.name);
            Ok(())
        }
    }
}

fn run_started_components() -> Vec<Component> {
    vec![
        Component::ActionRow(ActionRow {
        components: vec![
            interactions::button_component(
                "Finish run",
                CUSTOM_ID_FINISH_RUN,
                ButtonStyle::Success,
            ),
            interactions::button_component(
                "Forfeit",
                CUSTOM_ID_FORFEIT_RUN,
                ButtonStyle::Danger,
            ),
        ],
    })]
}

fn run_started_interaction_response(race_run: &RaceRun, preamble: Option<&str>) -> Result<InteractionResponse, String> {
    let filenames = race_run.filenames()?;
    let content = if let Some(p) = preamble {
        format!("{}

Good luck! your filenames are: `{}`

If anything goes wrong, tell an admin there was an issue with run `{}`
", p, filenames, race_run.uuid)
    } else {
        format!("Good luck! your filenames are: `{}`

If anything goes wrong, tell an admin there was an issue with run `{}`
", filenames, race_run.uuid)
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
) -> Result<(), ErrorResponse> {
    const USER_FACING_ERROR: &str = "There was an error starting your race. Please ping FoxLisk.";
    let mut rr = RaceRun::get_by_message_id(interaction.message.id, &state.pool)
        .await
        .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?;
    rr.start();
    match rr.save(&state.pool).await {
        Ok(_) => {
            let resp = run_started_interaction_response(&rr, None)
                .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?;
            state
                .interaction_client()
                .create_response(interaction.id, &interaction.token, &resp)
                .exec()
                .await
                .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))
                .map(|_| ())
        }
        Err(e) => {
            Err(ErrorResponse::new(USER_FACING_ERROR, format!("Error updating race run: {}", e)))
        }
    }
}

async fn update_race_run<M, T, F>(
    interaction: &T,
    f: F,
    state: &Arc<DiscordState>,
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

    let rro = match RaceRun::search_by_message_id(mid.clone(), &state.pool).await {
        Ok(r) => r,
        Err(e) => {
            return Err(e);
        }
    };
    match rro {
        Some(mut rr) => {
            f(&mut rr);
            {
                if let Err(e) = rr.save(&state.pool).await {
                    Err(format!("Error saving race {}: {}", rr.id, e))
                } else {
                    Ok(())
                }
            }
        }
        None => Err(format!("Update for unknown race with message id {}", mid)),
    }
}

async fn handle_run_forfeit_button(
    interaction: Box<MessageComponentInteraction>,
    state: &Arc<DiscordState>,
) -> Result<(), ErrorResponse> {
    const USER_FACING_ERROR: &str =
        "Something went wrong forfeiting this match. Please ping FoxLisk.";

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
    state
        .interaction_client()
        .create_response(interaction.id, &interaction.token, &ir)
        .exec()
        .await
        .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))
        .map(|_| ())
}


lazy_static! {
    static ref FORFEIT_REGEX: regex::Regex =
        regex::RegexBuilder::new(r"\s*forfeit\s*").case_insensitive(true).build().unwrap();
}

async fn handle_run_forfeit_modal(
    mut interaction: Box<ModalSubmitInteraction>,
    state: &Arc<DiscordState>,
) -> Result<(), ErrorResponse> {
    const USER_FACING_ERROR: &str = "Something went wrong forfeiting this race. Please ping FoxLisk.";
    let ut = get_field_from_modal_components(
        std::mem::take(&mut interaction.data.components),
        CUSTOM_ID_FORFEIT_MODAL_INPUT,
    )
        .ok_or(ErrorResponse::new(
            USER_FACING_ERROR,
            "Error getting forfeit input from modal.",
        ))?;
    let ir = if FORFEIT_REGEX.is_match(&ut) {
        update_race_run(&interaction, |rr| rr.forfeit(), state)
            .await
            .map_err(|e| {
                ErrorResponse::new(
                    USER_FACING_ERROR,
                    e,
                )
            })?;

        update_resp_to_plain_content(
            "You have forfeited this match. Please let the admins know if there are any issues.")
    } else {
        let mid = match interaction.get_message_id() {
            Some(id) => id,
            None => {
                return Err(
                    ErrorResponse::new(USER_FACING_ERROR,
                    format!("Interaction {:?} has no message id", interaction)
                    )
                );
            }
        };

        RaceRun::get_by_message_id(mid, &state.pool).await
            .and_then(|race_run| run_started_interaction_response(&race_run, Some("Forfeit canceled")))
            .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))?
    };
    state
        .interaction_client()
        .create_response(interaction.id, &interaction.token, &ir)
        .exec()
        .await
        .map_err(|e| {
            ErrorResponse::new(
                USER_FACING_ERROR,
                e.to_string(),
            )
        })
        .map(|_| ())
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
            components: Some(vec![Component::ActionRow(ActionRow { components })]),
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
) -> Result<(), ErrorResponse> {
    const USER_FACING_ERROR: &str = "Something went wrong finishing this run. Please ping FoxLisk.";
    if let Err(e) = update_race_run(
        &interaction,
        |rr| {
            rr.finish();
        },
        state,
    )
    .await
    {
        return Err(ErrorResponse::new(USER_FACING_ERROR, e));
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

    state
        .interaction_client()
        .create_response(interaction.id, &interaction.token, &ir)
        .exec()
        .await
        .map_err(|e| ErrorResponse::new(USER_FACING_ERROR, e))
        .map(|_| ())
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
) -> Result<(), ErrorResponse> {
    let ut = get_field_from_modal_components(
        std::mem::take(&mut interaction.data.components),
        CUSTOM_ID_USER_TIME,
    )
    .ok_or(ErrorResponse::new(
        "Something went wrong reporting your time. Please ping FoxLisk.",
        "Error getting user time form modal.",
    ))?;
    update_race_run(&interaction, |rr| rr.report_user_time(ut), state)
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
            components: Some(vec![Component::ActionRow(ActionRow {
                components: vec![interactions::button_component(
                    "VoD ready",
                    CUSTOM_ID_VOD_READY,
                    ButtonStyle::Success,
                )],
            })]),
            content: Some("Please click here once your VoD is ready".to_string()),
            ..Default::default()
        }),
    };
    state
        .interaction_client()
        .create_response(interaction.id, &interaction.token, &ir)
        .exec()
        .await
        .map_err(|e| {
            ErrorResponse::new(
                "Something went wrong reporting your time. Please ping FoxLisk.",
                e.to_string(),
            )
        })
        .map(|_| ())
}

async fn handle_vod_ready(
    interaction: Box<MessageComponentInteraction>,
    state: &Arc<DiscordState>,
) -> Result<(), ErrorResponse> {
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
    state
        .interaction_client()
        .create_response(interaction.id, &interaction.token, &ir)
        .exec()
        .await
        .map_err(|e| {
            ErrorResponse::new(
                "There was an error accepting your VoD. Please ping FoxLisk",
                e,
            )
        })
        .map(|_| ())
}

async fn handle_vod_modal(
    mut interaction: Box<ModalSubmitInteraction>,
    state: &Arc<DiscordState>,
) -> Result<(), ErrorResponse> {
    let user_input = get_field_from_modal_components(
        std::mem::take(&mut interaction.data.components),
        CUSTOM_ID_VOD_MODAL_INPUT,
    )
    .ok_or(ErrorResponse::new(
        "Something went wrong reporting your VoD. Please ping FoxLisk.",
        "Error getting vod from modal.",
    ))?;
    update_race_run(&interaction, |rr| rr.set_vod(user_input), state)
        .await
        .map_err(|e| {
            ErrorResponse::new(
                "Something went wrong reporting your VoD. Please ping FoxLisk.",
                e,
            )
        })?;
    let ir = interactions::plain_interaction_response(
        "Thank you, your race is completed. Please message the admins if there are any issues.",
    );

    state
        .interaction_client()
        .create_response(interaction.id.clone(), &interaction.token, &ir)
        .exec()
        .await
        .map_err(|e| {
            ErrorResponse::new(
                "Something went wrong reporting your VoD. Please ping FoxLisk.",
                e.to_string(),
            )
        })
        .map(|_| ())
}

async fn handle_button_interaction(
    interaction: Box<MessageComponentInteraction>,
    state: &Arc<DiscordState>,
) -> Result<(), ErrorResponse> {
    match interaction.data.custom_id.as_str() {
        CUSTOM_ID_START_RUN => handle_run_start(interaction, state).await,
        CUSTOM_ID_FORFEIT_RUN => handle_run_forfeit_button(interaction, state).await,
        CUSTOM_ID_FINISH_RUN => handle_run_finish(interaction, state).await,
        CUSTOM_ID_VOD_READY => handle_vod_ready(interaction, state).await,
        _ => {
            println!("Unhandled button: {:?}", interaction);
            Ok(())
        }
    }
}

async fn handle_modal_submission(
    interaction: Box<ModalSubmitInteraction>,
    state: &Arc<DiscordState>,
) -> Result<(), ErrorResponse> {
    match interaction.data.custom_id.as_str() {
        CUSTOM_ID_USER_TIME_MODAL => handle_user_time_modal(interaction, state).await,
        CUSTOM_ID_VOD_MODAL => handle_vod_modal(interaction, state).await,
        CUSTOM_ID_FORFEIT_MODAL => handle_run_forfeit_modal(interaction, state).await,
        _ => {
            println!("Unhandled modal: {:?}", interaction);
            Ok(())
        }
    }
}

async fn _handle_interaction(
    interaction: InteractionCreate,
    state: &Arc<DiscordState>,
) -> Result<(), ErrorResponse> {
    match interaction.0 {
        Interaction::ApplicationCommand(ac) => handle_application_interaction(ac, &state).await,
        Interaction::MessageComponent(mc) => handle_button_interaction(mc, &state).await,
        Interaction::ModalSubmit(ms) => handle_modal_submission(ms, &state).await,
        _ => {
            println!("Unhandled interaction: {:?}", interaction);
            Ok(())
        }
    }
}

async fn handle_interaction(interaction: InteractionCreate, state: Arc<DiscordState>) {
    let interaction_id = interaction.id();
    let token = interaction.token().to_string();
    if let Err(e) = _handle_interaction(interaction, &state).await {
        // inform user about the error
        let ir = update_resp_to_plain_content(e.user_facing_error.clone());
        let err_ext = state
            .interaction_client()
            .create_response(interaction_id, &token, &ir)
            .exec()
            .await
            .map_err(|e| e.to_string())
            .map(|_| ())
            .err();
        if e.internal_error.is_some() || err_ext.is_some() {
            // inform admins about the error
            let final_internal_error = format!(
                "Error: {} - Internal error: {:?} | Error communicating with user: {:?}",
                e.user_facing_error, e.internal_error, err_ext
            );
            println!("{}", final_internal_error);
            if let Err(e) = state.webhooks.message_async(&final_internal_error).await {
                // at some point you just have to give up
                println!("Error reporting internal error: {}", e);
            }
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
    ).default_member_permissions(Permissions::MANAGE_GUILD)
        .option(CommandOption::Integer(NumberCommandOptionData {
            autocomplete: false,
            choices: vec![],
            description: format!("Race ID. Get this from {}", WEBSITE_URL),
            description_localizations: None,
            max_value: None,
            min_value: None,
            name: "race_id".to_string(),
            name_localizations: None,
            required: true
        }
        ))
        .build();

    let commands = vec![create_race, cancel_race];

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

    match resp.model().await {
        Ok(cmds) => {
            Ok(())
        }
        Err(e) => {
            println!("Error inspecting list of returned commands: {}", e);
            Ok(())
        }
    }
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
