use crate::schema::players;
use crate::{save_fn, update_fn};
use diesel::prelude::*;
use diesel::SqliteConnection;
use serde::Deserialize;
use std::collections::HashMap;
use std::num::ParseIntError;
use std::str::FromStr;
use twilight_mention::Mention;
use twilight_model::id::marker::UserMarker;
use twilight_model::id::Id;
use twilight_model::user::User;

pub trait MentionOptional {
    fn mention_maybe(&self) -> Option<String>;
}

#[derive(Queryable, Debug, Clone, Identifiable, AsChangeset, serde::Serialize)]
pub struct Player {
    pub id: i32,
    /// display name
    pub name: String,
    pub discord_id: String,
    pub racetime_username: Option<String>,
    pub twitch_user_login: Option<String>,
    pub racetime_user_id: Option<String>,
}

impl Player {
    /// this should never fail but i'm scared of assuming that
    pub fn discord_id(&self) -> Result<Id<UserMarker>, ParseIntError> {
        Id::<UserMarker>::from_str(&self.discord_id)
    }

    pub fn by_id(
        ids: Option<Vec<i32>>,
        conn: &mut SqliteConnection,
    ) -> Result<HashMap<i32, Player>, diesel::result::Error> {
        let ps = if let Some(ids) = ids {
            players::table.filter(players::id.eq_any(ids)).load(conn)?
        } else {
            players::table.load(conn)?
        };
        Ok(ps.into_iter().map(|p: Player| (p.id, p)).collect::<_>())
    }

    pub fn get_by_discord_id(
        id: &str,
        conn: &mut SqliteConnection,
    ) -> Result<Option<Self>, diesel::result::Error> {
        players::table
            .filter(players::discord_id.eq(id))
            .first(conn)
            .optional()
    }

    /// returns (Player, was_just_created)
    pub fn get_or_create_from_discord_user(
        user: User,
        conn: &mut SqliteConnection,
    ) -> Result<(Self, bool), diesel::result::Error> {
        if let Some(u) = Self::get_by_discord_id(&user.id.to_string(), conn)? {
            Ok((u, false))
        } else {
            // uhh... it'd be nice to have user's server nick here but w/e
            let best_name = user.global_name.unwrap_or(user.name);
            let np = NewPlayer::new(best_name, user.id.to_string(), None, None);
            let saved = np.save(conn)?;
            Ok((saved, true))
        }
    }

    pub fn get_by_id(
        id: i32,
        conn: &mut SqliteConnection,
    ) -> Result<Option<Self>, diesel::result::Error> {
        players::table.find(id).first(conn).optional()
    }
    pub fn get_by_rtgg_id(
        rtgg_id: &str,
        conn: &mut SqliteConnection,
    ) -> Result<Option<Self>, diesel::result::Error> {
        players::table
            .filter(players::racetime_user_id.eq(rtgg_id))
            .first(conn)
            .optional()
    }

    pub fn get_by_name(
        name: &str,
        conn: &mut SqliteConnection,
    ) -> Result<Option<Self>, diesel::result::Error> {
        players::table
            .filter(players::name.eq(name))
            .first(conn)
            .optional()
    }

    pub fn mention_or_name(&self) -> String {
        self.discord_id()
            .ok()
            .map(|i| i.mention().to_string())
            .unwrap_or(self.name.clone())
    }
    update_fn! {}
}

impl<T> MentionOptional for Result<Option<Player>, T> {
    fn mention_maybe(&self) -> Option<String> {
        match self {
            Ok(o) => o
                .as_ref()
                .and_then(|i| i.discord_id().ok())
                .map(|i| i.mention().to_string()),
            Err(_e) => None,
        }
    }
}

#[derive(Insertable, Deserialize, Debug)]
#[diesel(table_name=players)]
pub struct NewPlayer {
    pub name: String,
    pub discord_id: String,
    pub racetime_username: Option<String>,
    pub twitch_user_login: Option<String>,
}

impl NewPlayer {
    pub fn new<S: Into<String>>(
        name: S,
        discord_id: S,
        racetime_username: Option<S>,
        twitch_user_login: Option<S>,
    ) -> Self {
        Self {
            name: name.into(),
            discord_id: discord_id.into(),
            racetime_username: racetime_username.map(Into::into),
            twitch_user_login: twitch_user_login.map(Into::into),
        }
    }
    save_fn!(players::table, Player);
}
