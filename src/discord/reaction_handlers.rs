use std::collections::HashSet;
use std::ops::DerefMut;
use std::sync::Arc;
use bb8::RunError;
use diesel::{ConnectionError, SqliteConnection};
use twilight_http::request::channel::reaction::RequestReactionType;
use twilight_http::response::DeserializeBodyError;
use twilight_model::channel::message::MessageReaction;
use twilight_model::channel::ReactionType;
use twilight_model::gateway::payload::incoming::ReactionAdd;
use nmg_league_bot::models::bracket_race_infos::BracketRaceInfo;
use crate::discord::discord_state::DiscordState;

pub async fn handle_reaction_add(reaction: Box<ReactionAdd>, state: &Arc<DiscordState>) {
    println!("Handling new reaction: {:?}", reaction);
    match _handle_reaction_add(reaction, state).await {
        Ok(_) => {},
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

impl From<DeserializeBodyError> for ReactionAddError {
    fn from(e: DeserializeBodyError) -> Self {
        Self::DeserializeBodyError(e)
    }
}

async fn _handle_reaction_add(reaction: Box<ReactionAdd>, state: &Arc<DiscordState>) -> Result<(), ReactionAddError> {
    let member = reaction.member.as_ref().ok_or(ReactionAddError::InexplicablyMissingMember)?;
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

    let info = match BracketRaceInfo::get_by_commportunities_message_id(reaction.message_id, conn)? {
        Some(i) => i,
        None => {return Ok(()); }
    };
    println!("Handling commportunity signup!");

    // okay this is hella stupid
    // to get all reactions on a message, you have to get the message first, which gives you
    // a list of reaction emoji + counts. you have to do further queries to see who did the
    // reactions
    //
    // we could in theory use the gateway cache for this but... i don't really trust it?
    // i don't want people who sign up 2 days apart with a bot restart in between to get
    // missed
    let msg = state.client.message(reaction.channel_id, reaction.message_id)
        .exec()
        .await?
        .model()
        .await?;
    let mut non_self_reactions = HashSet::new();
    for e in msg.reactions {
        let rrt = request_reaction_type_from_message_reaction(&e.emoji);
        let rxns = state.client.reactions(msg.channel_id, msg.id, &rrt)
            .exec()
            .await?
            .models()
            .await?;
        // N.B. i really want to exclude the bot's _own_ reactions, but this is very simple
        // and, i think, probably more correct anyway - if dao wants to write a bot to
        // automatically volunteer for comms, he can take it up with me first :)
        non_self_reactions.extend(rxns.into_iter().filter(|r| ! r.bot));
    }
    println!("non_self_reactions reactions on post: {:?}", non_self_reactions);

    if non_self_reactions.len() >= 2 {
        // create sirius-inbox post
        // create zsr post
    }


    Ok(())

}

fn request_reaction_type_from_message_reaction(rt: &ReactionType) -> RequestReactionType {
    match rt {
        ReactionType::Custom { animated, id, name } => {
            RequestReactionType::Custom {
                id: id.clone(),
                name: name.as_ref().map(|s| s.as_str())
            }
        }
        ReactionType::Unicode { name } => {
            RequestReactionType::Unicode {name}
        }
    }
}