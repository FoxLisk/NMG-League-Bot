use bb8::RunError;
use diesel::{ConnectionError, SqliteConnection};
use itertools::Itertools;
use std::collections::HashSet;
use std::ops::DerefMut;
use std::sync::Arc;
use twilight_http::request::channel::reaction::RequestReactionType;
use twilight_http::response::DeserializeBodyError;
use twilight_mention::timestamp::{Timestamp, TimestampStyle};
use twilight_mention::Mention;
use twilight_model::channel::embed::{Embed, EmbedField};
use twilight_model::channel::message::MessageReaction;
use twilight_model::channel::{Message, ReactionType};
use twilight_model::gateway::payload::incoming::ReactionAdd;
use twilight_model::id::marker::MessageMarker;
use twilight_model::id::Id;
use twilight_model::user::User;
use twilight_util::builder::embed::EmbedFooterBuilder;
use twilight_validate::message::MessageValidationError;

use crate::discord::discord_state::DiscordState;
use nmg_league_bot::models::bracket_race_infos::BracketRaceInfo;

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

    let mut info =
        match BracketRaceInfo::get_by_commportunities_message_id(reaction.message_id, conn)? {
            Some(i) => handle_commportunities_reaction(i, reaction, state).await,
            None => {
                return Ok(());
            }
        };

    Ok(())
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
    info: BracketRaceInfo,
    reaction: Box<ReactionAdd>,
    state: &Arc<DiscordState>,
) -> Result<(), ReactionAddError> {
    println!("[not] handling commentary confirmation!");
    todo!()
}

async fn handle_commentary_signup(
    info: BracketRaceInfo,
    reaction: Box<ReactionAdd>,
    state: &Arc<DiscordState>,
) -> Result<(), ReactionAddError> {
    // okay this is hella stupid
    // to get all reactions on a message, you have to get the message first, which gives you
    // a list of reaction emoji + counts. you have to do further queries to see who did the
    // reactions
    //
    // we could in theory use the gateway cache for this but... i don't really trust it?
    // i don't want people who sign up 2 days apart with a bot restart in between to get
    // missed
    let mut msg = state
        .client
        .message(reaction.channel_id, reaction.message_id)
        .exec()
        .await?
        .model()
        .await?;
    let mut non_self_reactions = HashSet::new();
    for e in msg.reactions.drain(..) {
        let rrt = request_reaction_type_from_message_reaction(&e.emoji);
        let rxns = state
            .client
            .reactions(msg.channel_id, msg.id, &rrt)
            .exec()
            .await?
            .models()
            .await?;
        // N.B. i really want to exclude the bot's _own_ reactions, but this is very simple
        // and, i think, probably more correct anyway - if dao wants to write a bot to
        // automatically volunteer for comms, he can take it up with me first :)
        non_self_reactions.extend(rxns.into_iter().filter(|r| !r.bot));
    }
    if non_self_reactions.len() >= 2 {
        let m = create_sirius_inbox_post(&reaction, non_self_reactions, &info, state).await?;
    }
    Ok(())
}

async fn create_sirius_inbox_post(
    reaction: &Box<ReactionAdd>,
    users: HashSet<User>,
    info: &BracketRaceInfo,
    state: &Arc<DiscordState>,
) -> Result<Message, ReactionAddError> {
    let when = match info.scheduled() {
        Some(ts) => {
            let long = Timestamp::new(ts.timestamp() as u64, Some(TimestampStyle::LongDateTime));
            let short = Timestamp::new(ts.timestamp() as u64, Some(TimestampStyle::RelativeTime));
            format!("{} ({})", long.mention(), short.mention())
        }
        None => "ERROR: can't find scheduled time.".to_string(),
    };

    let mut cxn = state.diesel_cxn().await?;
    let conn = cxn.deref_mut();
    let race = info.race(conn)?;
    let (p1, p2) = race.players(conn)?;

    let wh = state.webhooks.prepare_execute_sirius_inbox();
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
        description: Some(format!(
            "{when} {} vs {}",
            p1.mention_or_name(),
            p2.mention_or_name()
        )),
        fields,
        footer: None,
        image: None,
        kind: "rich".to_string(),
        provider: None,
        thumbnail: None,
        timestamp: None,
        title: Some(format!("Commentator Signup")),
        url: None,
        video: None,
    }];
    wh.embeds(&embeds)?
        .wait()
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
        ReactionType::Custom { animated, id, name } => RequestReactionType::Custom {
            id: id.clone(),
            name: name.as_ref().map(|s| s.as_str()),
        },
        ReactionType::Unicode { name } => RequestReactionType::Unicode { name },
    }
}
