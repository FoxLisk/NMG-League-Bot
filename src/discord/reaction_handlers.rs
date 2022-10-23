use bb8::RunError;
use diesel::{ConnectionError};
use itertools::Itertools;
use std::collections::{HashMap, HashSet};
use std::ops::DerefMut;
use std::sync::Arc;
use twilight_http::request::channel::reaction::RequestReactionType;
use twilight_http::response::DeserializeBodyError;
use twilight_mention::Mention;
use twilight_model::channel::embed::{Embed, EmbedField};
use twilight_model::channel::{Message, ReactionType};
use twilight_model::gateway::payload::incoming::ReactionAdd;
use twilight_model::id::marker::{ ScheduledEventMarker};
use twilight_model::id::Id;
use twilight_model::user::User;
use twilight_validate::message::MessageValidationError;

use crate::discord::discord_state::DiscordState;
use nmg_league_bot::models::bracket_race_infos::BracketRaceInfo;
use crate::discord::race_to_nice_embeds;

pub async fn handle_reaction_add(reaction: Box<ReactionAdd>, state: &Arc<DiscordState>) {
    println!("Handling new reaction: {:?}", reaction);
    match _handle_reaction_add(reaction, state).await {
        Ok(_) => {}
        Err(e) => {
            println!("Error processing reaction add {:?}", e);
        }
    }
}

#[derive(Debug)]
enum ReactionAddError {
    DatabaseError(diesel::result::Error),
    RunError(bb8::RunError<ConnectionError>),
    InexplicablyMissingMember,
    HttpError(twilight_http::Error),
    DeserializeBodyError(DeserializeBodyError),
    ValidationError(MessageValidationError),
}

impl From<diesel::result::Error> for ReactionAddError {
    fn from(e: diesel::result::Error) -> Self {
        Self::DatabaseError(e)
    }
}

impl From<RunError<ConnectionError>> for ReactionAddError {
    fn from(e: RunError<ConnectionError>) -> Self {
        Self::RunError(e)
    }
}

impl From<twilight_http::Error> for ReactionAddError {
    fn from(e: twilight_http::Error) -> Self {
        Self::HttpError(e)
    }
}

impl From<twilight_validate::message::MessageValidationError> for ReactionAddError {
    fn from(e: MessageValidationError) -> Self {
        Self::ValidationError(e)
    }
}

impl From<DeserializeBodyError> for ReactionAddError {
    fn from(e: DeserializeBodyError) -> Self {
        Self::DeserializeBodyError(e)
    }
}

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
            println!("This is a reaction I created tho");
            return Ok(());
        }
    }
    // TODO: state should maintain state about interesting message IDs
    //       we're gonna be doing a bunch of stupid table scans for now though
    let mut _conn = state.diesel_cxn().await?;
    let conn = _conn.deref_mut();


    match BracketRaceInfo::get_by_commportunities_message_id(reaction.message_id, conn)? {
        Some(i) => handle_commportunities_reaction(i, reaction, state).await,
        None => {
            println!("Uninteresting reaction");
            Ok(())
        }
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
            println!(
                "Error checking if the user for reaction {:?} is an admin: {}",
                reaction, e
            );
            return false;
        }
    };
    match &reaction.emoji {
        ReactionType::Custom { name, .. } => {
            return name.as_ref().map(|s| s == "Linkbot").unwrap_or(false)
        }
        ReactionType::Unicode { .. } => false,
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

async fn handle_commentary_confirmation(
    mut info: BracketRaceInfo,
    reaction: Box<ReactionAdd>,
    state: &Arc<DiscordState>,
) -> Result<(), ReactionAddError> {
    let all_rxns = all_reactions(&reaction, state).await?;
    let commentators: HashSet<User> = all_rxns
        .into_values()
        .flatten()
        .filter(|r| !r.bot)
        .collect();
    if let Some(gse) = info.get_scheduled_event_id() {
        if let Err(e) = update_scheduled_event(gse, &commentators, state).await {
            println!("Error updating scheduled event: {:?}", e);
        }
    }

    let mut cxn = state.diesel_cxn().await?;
    let conn = cxn.deref_mut();

    // we're sending almost identical messages to zsr & commentary-discussion
    let mut fields = race_to_nice_embeds(&info, conn)?;

    let comms = commentators.iter().map(|c| c.id.mention()).join(" and ");
    fields.push(
        EmbedField {
            inline: false,
            name: "Commentators".to_string(),
            value: comms,
        },
    );

    let fields = race_to_nice_embeds(&info, conn)?;

    if let Err(e) = create_tentative_commentary_discussion_post(fields.clone(), state).await {
        println!("Error creating commentary discussion post: {:?}", e);
    }
    match create_restream_request_post(fields.clone(), state).await {
        Ok(m) => {
            info.set_restream_request_message_id(m.id);
        },
        Err(e) => {
            println!("Error creating restream request post: {:?}", e);
        }
    }
    if let Some(commp_msg_id) = info.get_commportunities_message_id() {
        if let Err(e) = state.client.delete_message(
            state.channel_config.commportunities,
            commp_msg_id
        )
            .exec()
            .await {
            println!("Error deleting commportunities message: {}", e);
        } else {
            info.clear_commportunities_message_id();
        }
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
    state.client.create_message(state.channel_config.commentary_discussion.clone())
    .embeds(&embeds)?
        .exec()
        .await?
        .model()
        .await
        .map_err(From::from)

}

async fn create_restream_request_post(fields: Vec<EmbedField>, state: &Arc<DiscordState>) -> Result<Message, ReactionAddError> {
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
        title: Some("Restream Channel Request".to_string()),
        url: None,
        video: None,
    }];
    state.client.create_message(state.channel_config.zsr.clone())
        .embeds(&embeds)?
        .exec()
        .await?
        .model()
        .await
        .map_err(From::from)
}

async fn update_scheduled_event(
    gse_id: Id<ScheduledEventMarker>,
    commentators: &HashSet<User>,
    state: &Arc<DiscordState>,
) -> Result<(), ReactionAddError> {
    println!("[TODO] update scheduled event");
    Ok(())
}

async fn handle_commentary_signup(
    info: BracketRaceInfo,
    reaction: Box<ReactionAdd>,
    state: &Arc<DiscordState>,
) -> Result<(), ReactionAddError> {
    let all_rxns = all_reactions(&reaction, state).await?;
    let non_self_reactions: HashSet<User> = all_rxns
        .into_values()
        .flatten()
        .filter(|r| !r.bot)
        .collect();
    if non_self_reactions.len() >= 2 {
        create_sirius_inbox_post(&reaction, non_self_reactions, &info, state).await?;
    }
    Ok(())
}

async fn create_sirius_inbox_post(
    reaction: &Box<ReactionAdd>,
    users: HashSet<User>,
    info: &BracketRaceInfo,
    state: &Arc<DiscordState>,
) -> Result<Message, ReactionAddError> {
    let mut cxn = state.diesel_cxn().await?;
    let conn = cxn.deref_mut();
    let when = info
        .scheduled_time_formatted()
        .unwrap_or("ERROR: can't find scheduled time.".to_string());
    let title = info.race(conn)?.title(conn)?;

    let names = users.iter().map(|u| u.id.mention().to_string()).join(", ");
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

    state.client.create_message(state.channel_config.sirius_inbox.clone())
        .embeds(&embeds)?
        .exec()
        .await?
        .model()
        .await
        .map_err(From::from)
}

fn reaction_message_url(r: &Box<ReactionAdd>) -> Option<String> {
    Some(format!(
        "https://discord.com/channels/{}/{}/{}",
        r.guild_id?, r.channel_id, r.message_id
    ))
}

fn request_reaction_type_from_message_reaction(rt: &ReactionType) -> RequestReactionType {
    match rt {
        ReactionType::Custom { id, name, .. } => RequestReactionType::Custom {
            id: id.clone(),
            name: name.as_ref().map(|s| s.as_str()),
        },
        ReactionType::Unicode { name } => RequestReactionType::Unicode { name },
    }
}

async fn all_reactions(
    reaction: &Box<ReactionAdd>,
    state: &Arc<DiscordState>,
) -> Result<HashMap<ReactionType, Vec<User>>, ReactionAddError> {
    // okay this is hella stupid
    // to get all reactions on a message, you have to get the message first, which gives you
    // a list of reaction emoji + counts. you have to do further queries to see who did the
    // reactions
    //
    // we could in theory use the gateway cache for this but... i don't really trust it?
    // i don't want people who sign up 2 days apart with a bot restart in between to get
    // missed
    let mut rxns = HashMap::new();
    let msg = state
        .client
        .message(reaction.channel_id, reaction.message_id)
        .exec()
        .await?
        .model()
        .await?;
    for e in msg.reactions {
        let rrt = request_reaction_type_from_message_reaction(&e.emoji);
        let emoji_reactions = state
            .client
            .reactions(msg.channel_id, msg.id, &rrt)
            .exec()
            .await?
            .models()
            .await?;
        rxns.insert(e.emoji, emoji_reactions);
    }

    Ok(rxns)
}
