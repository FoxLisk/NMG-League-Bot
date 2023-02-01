use std::num::ParseIntError;
use std::str::FromStr;
use crate::save_fn;
use crate::schema::players;
use diesel::prelude::*;
use diesel::SqliteConnection;
use twilight_mention::Mention;
use twilight_model::id::Id;
use twilight_model::id::marker::UserMarker;
use serde::Deserialize;

pub trait MentionOptional {
    fn mention_maybe(&self) -> Option<String>;
}

#[derive(Queryable, Debug, Clone)]
pub struct Player {
    pub id: i32,
    /// display name
    pub name: String,
    pub discord_id: String,
    pub racetime_username: Option<String>,
    pub twitch_user_login: Option<String>,
    pub restreams_ok: i32,
}

impl Player {
    /// this should never fail but i'm scared of assuming that
    pub fn discord_id(&self) -> Result<Id<UserMarker>, ParseIntError> {
        Id::<UserMarker>::from_str(&self.discord_id)
    }

    pub fn restreams_ok(&self) -> bool {
        self.restreams_ok == 1
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

    pub fn get_by_id(id: i32, conn: &mut SqliteConnection) -> Result<Option<Self>, diesel::result::Error> {
        players::table.find(id).first(conn).optional()
    }

    pub fn mention_or_name(&self) -> String {
        self.discord_id().ok().map(|i| i.mention().to_string()).unwrap_or(self.name.clone())
    }

}

impl<T> MentionOptional for Result<Option<Player>, T> {
    fn mention_maybe(&self) -> Option<String> {
        match self {
            Ok(o) => {
                o.as_ref().and_then(|i| i.discord_id().ok())
                    .map(|i| i.mention().to_string())
            }
            Err(_e) => {None}
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
    pub restreams_ok: i32,
}

impl NewPlayer {
    pub fn new<S: Into<String>>(
        name: S,
        discord_id: S,
        racetime_username: Option<S>,
        twitch_user_login: Option<S>,
        restreams_ok: bool,
    ) -> Self {
        Self {
            name: name.into(),
            discord_id: discord_id.into(),
            racetime_username: racetime_username.map(Into::into),
            twitch_user_login: twitch_user_login.map(Into::into),
            restreams_ok: if restreams_ok { 1 } else { 0 },
        }
    }
    save_fn!(players::table, Player);
}
