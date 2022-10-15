use crate::models::brackets::Bracket;
use crate::save_fn;
use crate::schema::players;
use diesel::prelude::*;
use diesel::SqliteConnection;

#[derive(Queryable, Debug)]
pub struct Player {
    pub id: i32,
    pub name: String,
    pub discord_id: String,
    pub racetime_username: String,
    restreams_ok: i32,
}

impl Player {
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
