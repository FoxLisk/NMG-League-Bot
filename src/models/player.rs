use diesel::prelude::Insertable;
// use crate::schema::players::dsl::*;
use crate::schema::players;

#[derive(Queryable)]
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
}

#[derive(Insertable)]
#[diesel(table_name=players)]
pub struct NewPlayer {
    pub name: String,
    pub discord_id: String,
    pub racetime_username: String,
    #[diesel(serialize_as=i32)]
    pub restreams_ok: bool,
}

// use
//
// struct
