use std::num::ParseIntError;
use std::str::FromStr;
use crate::save_fn;
use crate::schema::players;
use diesel::prelude::*;
use diesel::SqliteConnection;
use twilight_mention::Mention;
use twilight_model::id::Id;
use twilight_model::id::marker::UserMarker;

pub trait IntoMentionOptional {
    fn mention_maybe(self) -> Option<String>;
}

#[derive(Queryable, Debug)]
pub struct Player {
    pub id: i32,
    pub name: String,
    pub discord_id: String,
    pub racetime_username: String,
    restreams_ok: i32,
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
        Ok(players::table
            .filter(players::discord_id.eq(id))
            .load(conn)?
            .pop())
    }

    pub fn get_by_id(id: i32, conn: &mut SqliteConnection) -> Result<Option<Self>, diesel::result::Error> {
        players::table.find(id).first(conn).optional()
    }
}

impl<T> IntoMentionOptional for Result<Option<Player>, T> {
    fn mention_maybe(self) -> Option<String> {
        self.ok()
            .flatten()
            .and_then(|p| p.discord_id().ok())
            .map(|i| i.mention().to_string())
    }
}

#[derive(Insertable)]
#[diesel(table_name=players)]
pub struct NewPlayer {
    pub name: String,
    pub discord_id: String,
    pub racetime_username: String,
    pub restreams_ok: i32,
}

impl NewPlayer {
    pub fn new<S: Into<String>>(
        name: S,
        discord_id: S,
        racetime_username: S,
        restreams_ok: bool,
    ) -> Self {
        Self {
            name: name.into(),
            discord_id: discord_id.into(),
            racetime_username: racetime_username.into(),
            restreams_ok: if restreams_ok { 1 } else { 0 },
        }
    }
    save_fn!(players::table, Player);
}
