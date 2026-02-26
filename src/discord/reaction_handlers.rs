use crate::discord::discord_state::DiscordOperations;
use diesel::ConnectionError;
use itertools::Itertools;
use log::{debug, warn};
use std::ops::DerefMut;
use std::sync::Arc;
use thiserror::Error;
use twilight_http::response::DeserializeBodyError;
use twilight_mention::Mention;
use twilight_model::channel::message::embed::EmbedField;
use twilight_model::channel::message::{AllowedMentions, Embed, EmojiReactionType, MentionType};
use twilight_model::channel::Message;
use twilight_model::gateway::payload::incoming::{ReactionAdd, ReactionRemove};
use twilight_model::id::marker::UserMarker;
use twilight_model::id::Id;
use twilight_validate::message::MessageValidationError;

use crate::discord::discord_state::{DiscordState, DiscordStateError};
use crate::discord::{
    clear_commportunities_message, clear_tentative_commentary_assignment_message,
};
use nmg_league_bot::models::bracket_race_infos::BracketRaceInfo;
use nmg_league_bot::utils::race_to_nice_embeds;

use super::{comm_ids_and_names, embed_with_title};

pub async fn handle_reaction_remove(reaction: Box<ReactionRemove>, state: &Arc<DiscordState>) {
    debug!("Handling reaction removed: {:?}", reaction);
    if let Err(e) = _handle_reaction_remove(reaction, state).await {
        warn!("Error handling removed reaction: {:?}", e);
    }
}

pub async fn handle_reaction_add(reaction: Box<ReactionAdd>, state: &Arc<DiscordState>) {
    debug!("Handling new reaction: {:?}", reaction);
    match _handle_reaction_add(reaction, state).await {
        Ok(_) => {}
        Err(e) => {
            warn!("Error processing reaction add {:?}", e);
        }
    }
}

#[derive(Debug, Error)]
enum ReactionAddError {
    #[error("Error running database query: {0}")]
    DatabaseError(#[from] diesel::result::Error),
    #[error("Error getting a database connection: {0}")]
    RunError(#[from] bb8::RunError<ConnectionError>),
    #[error("Reaction had no member?")]
    InexplicablyMissingMember,
    #[error("Something went wrong checking for roles or whatever: {0}")]
    DiscordStateError(#[from] DiscordStateError),
    #[error("HTTP Error: {0}")]
    HttpError(#[from] twilight_http::Error),
    #[error("Error deserializing a response body: {0}")]
    DeserializeBodyError(#[from] DeserializeBodyError),
    #[error("Error validating a message: {0}")]
    MessageValidationError(#[from] MessageValidationError),
    #[error("Error validating a request: {0}")]
    RequestValidationError(#[from] twilight_validate::request::ValidationError),
}

async fn _handle_reaction_remove(
    reaction: Box<ReactionRemove>,
    state: &Arc<DiscordState>,
) -> Result<(), ReactionAddError> {
    let mut cxn = state.diesel_cxn().await?;
    if let Some(mut info) =
        BracketRaceInfo::get_by_commportunities_message_id(reaction.message_id, cxn.deref_mut())?
    {
        let res = info.remove_commentator(reaction.user_id, cxn.deref_mut())?;
        debug!("{} comms removed", res);
    } else {
        debug!("Uninteresting reaction removal");
    }
    Ok(())
}

// ENTRY POINT
async fn _handle_reaction_add(
    reaction: Box<ReactionAdd>,
    state: &Arc<DiscordState>,
) -> Result<(), ReactionAddError> {
    let member = reaction
        .member
        .as_ref()
        .ok_or(ReactionAddError::InexplicablyMissingMember)?;

    if let Some(cm) = state.cache.current_user() {
        if member.user.id == cm.id {
            debug!("Handling reaction from self - no-op");
            return Ok(());
        }
    }
    // TODO: state should maintain state about interesting message IDs
    //       we're gonna be doing a bunch of stupid table scans for now though
    let mut _conn = state.diesel_cxn().await?;
    let conn = _conn.deref_mut();

    if let Some(i) = BracketRaceInfo::get_by_commportunities_message_id(reaction.message_id, conn)?
    {
        handle_commportunities_reaction(i, reaction, state).await
    } else if let Some(i) =
        BracketRaceInfo::get_by_restream_request_message_id(reaction.message_id, conn)?
    {
        handle_restream_request_reaction(i, reaction, state).await
    } else {
        debug!("Uninteresting reaction {reaction:?}");
        Ok(())
    }
}

async fn is_admin_confirmation_reaction(
    reaction: &Box<ReactionAdd>,
    state: &Arc<DiscordState>,
) -> bool {
    let gid = match reaction.guild_id {
        Some(g) => g,
        None => {
            return false;
        }
    };
    match state.has_admin_role(reaction.user_id, gid).await {
        Ok(true) => {}
        Ok(false) => {
            return false;
        }
        Err(e) => {
            warn!("Error checking if the user for reaction {reaction:?} is an admin: {e}",);
            return false;
        }
    };
    match &reaction.emoji {
        EmojiReactionType::Custom { name, .. } => {
            return name.as_ref().map(|s| s == "Linkbot").unwrap_or(false)
        }
        EmojiReactionType::Unicode { .. } => false,
    }
}

async fn handle_commportunities_reaction(
    info: BracketRaceInfo,
    reaction: Box<ReactionAdd>,
    state: &Arc<DiscordState>,
) -> Result<(), ReactionAddError> {
    if is_admin_confirmation_reaction(&reaction, state).await {
        handle_commentary_confirmation(info, reaction, state).await
    } else {
        handle_commentary_signup(info, reaction, state).await
    }
}

// this is when an admin clicks Linkbot on a commportunities post, to indicate
// we are satisfied with the assigned commentators
async fn handle_commentary_confirmation(
    mut info: BracketRaceInfo,
    _reaction: Box<ReactionAdd>,
    state: &Arc<DiscordState>,
) -> Result<(), ReactionAddError> {
    let mut cxn = state.diesel_cxn().await?;
    let conn = cxn.deref_mut();
    let names: Vec<String> = comm_ids_and_names(&info, state, conn)
        .await?
        .into_iter()
        .map(|(_, name)| name)
        .collect();

    // we're sending almost identical messages to zsr & commentary-discussion
    let mut fields = race_to_nice_embeds(&info, conn)?;

    let comms_string = names.join(" and ");
    fields.push(EmbedField {
        inline: false,
        name: "Commentators".to_string(),
        value: comms_string,
    });

    match create_tentative_commentary_discussion_post(fields.clone(), state).await {
        Ok(m) => {
            info.set_tentative_commentary_assignment_message_id(m.id);
        }
        Err(e) => {
            warn!("Error creating commentary discussion post: {:?}", e);
        }
    }
    match create_restream_request_post(fields.clone(), state).await {
        Ok(m) => {
            info.set_restream_request_message_id(m.id);
        }
        Err(e) => {
            warn!("Error creating restream request post: {:?}", e);
        }
    }
    if let Err(e) =
        clear_commportunities_message(&mut info, &state.discord_client, &state.channel_config).await
    {
        warn!("Error clearing commportunities state: {e}");
    }

    info.update(conn)?;
    Ok(())
}

async fn create_tentative_commentary_discussion_post(
    fields: Vec<EmbedField>,
    state: &Arc<DiscordState>,
) -> Result<Message, ReactionAddError> {
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
        title: Some("Tentative Commentary Assignment".to_string()),
        url: None,
        video: None,
    }];
    state
        .discord_client
        .create_message(state.channel_config.commentary_discussion.clone())
        .embeds(&embeds)
        .await?
        .model()
        .await
        .map_err(From::from)
}

async fn create_restream_request_post(
    fields: Vec<EmbedField>,
    state: &Arc<DiscordState>,
) -> Result<Message, ReactionAddError> {
    let embeds = vec![embed_with_title(fields, "Restream Channel Request")];
    state
        .discord_client
        .create_message(state.channel_config.zsr.clone())
        .embeds(&embeds)
        .await?
        .model()
        .await
        .map_err(From::from)
}

async fn handle_commentary_signup(
    mut info: BracketRaceInfo,
    reaction: Box<ReactionAdd>,
    state: &Arc<DiscordState>,
) -> Result<(), ReactionAddError> {
    let mut cxn = state.diesel_cxn().await?;

    if info.new_commentator_signup(reaction.user_id, cxn.deref_mut())? {
        let commentators = info.commentator_signups(cxn.deref_mut())?;
        let ids = commentators
            .into_iter()
            .map(|c| c.discord_id())
            .flatten()
            .collect();
        create_commentator_signup_post(&reaction, ids, &info, state).await?;
    }
    Ok(())
}

async fn create_commentator_signup_post(
    reaction: &Box<ReactionAdd>,
    users: Vec<Id<UserMarker>>,
    info: &BracketRaceInfo,
    state: &Arc<DiscordState>,
) -> Result<Message, ReactionAddError> {
    let mut cxn = state.diesel_cxn().await?;
    let conn = cxn.deref_mut();
    let when = info
        .scheduled_time_formatted()
        .unwrap_or("ERROR: can't find scheduled time.".to_string());
    let title = info.race(conn)?.title(conn)?;

    let names = users.iter().map(|uid| uid.mention().to_string()).join(", ");
    let mut fields = vec![
        EmbedField {
            inline: false,
            name: "New signup".to_string(),
            value: reaction.user_id.mention().to_string(),
        },
        EmbedField {
            inline: false,
            name: "All Signups".to_string(),
            value: names,
        },
    ];
    if let Some(url) = reaction_message_url(reaction) {
        fields.push(EmbedField {
            inline: false,
            name: "Commportunities post".to_string(),
            value: url,
        })
    }
    let embeds = vec![Embed {
        author: None,
        color: Some(0x00b0f0),
        description: Some(format!("{when} {title}")),
        fields,
        footer: None,
        image: None,
        kind: "rich".to_string(),
        provider: None,
        thumbnail: None,
        timestamp: None,
        title: Some(format!("Race")),
        url: None,
        video: None,
    }];

    state
        .discord_client
        .create_message(state.channel_config.sirius_inbox.clone())
        .embeds(&embeds)
        .await?
        .model()
        .await
        .map_err(From::from)
}

async fn handle_restream_request_reaction(
    mut info: BracketRaceInfo,
    reaction: Box<ReactionAdd>,
    state: &Arc<DiscordState>,
) -> Result<(), ReactionAddError> {
    // TODO: ignore if the race is done or whatever
    let chan = match emoji_to_restream_channel(&reaction.emoji) {
        Some(c) => c,
        None => {
            return Ok(());
        }
    };
    let url = format!("https://twitch.tv/{chan}");
    info.restream_channel = Some(url.clone());

    let mut cxn = state.diesel_cxn().await?;
    let conn = cxn.deref_mut();

    let (comm_ids, comm_names): (Vec<Id<UserMarker>>, Vec<String>) =
        comm_ids_and_names(&info, state, conn)
            .await?
            .into_iter()
            .unzip();

    let mut fields = race_to_nice_embeds(&info, conn)?;
    fields.push(EmbedField {
        inline: false,
        name: "Commentators".to_string(),
        value: comm_names.join(" and "),
    });
    fields.push(EmbedField {
        inline: false,
        name: "Channel".to_string(),
        value: format!("https://twitch.tv/{chan}"),
    });

    let embeds = vec![Embed {
        author: None,
        color: Some(0x00b0f0),
        description: None,
        fields,
        footer: None,
        image: None,
        kind: "rich".to_string(),
        provider: None,
        thumbnail: None,
        timestamp: None,
        title: Some(format!("Commentary Assignment")),
        url: None,
        video: None,
    }];
    let (p1, p2) = info.race(conn)?.players(conn)?;
    let mut pings = comm_ids
        .iter()
        .map(|i| i.mention().to_string())
        .collect::<Vec<_>>();
    pings.push(p1.mention_or_name());
    pings.push(p2.mention_or_name());

    match state
        .discord_client
        .create_message(state.channel_config.commentary_discussion)
        .embeds(&embeds)
        .content(&pings.join(" "))
        .allowed_mentions(Some(&AllowedMentions {
            parse: vec![MentionType::Users],
            ..Default::default()
        }))
        .await
    {
        Ok(rm) => match rm.model().await {
            Ok(m) => {
                info.set_commentary_assignment_message_id(m.id);
            }
            Err(e) => {
                warn!("Error after creating commentary assignment message: {e}");
            }
        },
        Err(e) => {
            warn!(
                "Error creating commentary discussion message for bri {}: {e}",
                info.id
            );
        }
    }

    if let Err(e) = clear_tentative_commentary_assignment_message(
        &mut info,
        &state.discord_client,
        &state.channel_config,
    )
    .await
    {
        warn!("Error clearing tentative commentary assignment: {e}");
    }
    info.update(conn)?;

    Ok(())
}

fn emoji_to_restream_channel(rt: &EmojiReactionType) -> Option<&'static str> {
    match rt {
        EmojiReactionType::Custom { name, .. } => name
            .as_ref()
            .map(|s| if s == "greenham" { Some("FGfm") } else { None })
            .flatten(),
        EmojiReactionType::Unicode { name } => match name.as_str() {
            "1️⃣" => Some("zeldaspeedruns"),
            "2️⃣" => Some("zeldaspeedruns2"),
            "3️⃣" => Some("zeldaspeedruns_3"),
            "4️⃣" => Some("zeldaspeedruns_4"),
            _ => None,
        },
    }
}

fn reaction_message_url(r: &Box<ReactionAdd>) -> Option<String> {
    Some(format!(
        "https://discord.com/channels/{}/{}/{}",
        r.guild_id?, r.channel_id, r.message_id
    ))
}
