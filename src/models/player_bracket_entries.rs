use std::process::id;
use diesel::prelude::{Insertable, Queryable};
use diesel::{RunQueryDsl, SqliteConnection};
use crate::models::brackets::Bracket;

use crate::models::epoch_timestamp;
use crate::models::player::Player;
use crate::save_fn;
// use crate::schema::players::dsl::*;
use crate::schema::player_bracket_entry;

#[derive(Queryable, Debug)]
pub struct PlayerBracketEntry {
    pub id: i32,
    pub bracket_id: i32,
    pub player_id: i32,
}


#[derive(Insertable)]
#[diesel(table_name=player_bracket_entry)]
pub struct NewPlayerBracketEntry {
    pub bracket_id: i32,
    pub player_id: i32,
}

impl NewPlayerBracketEntry {
    pub fn new(bracket: &Bracket, player: &Player) -> Self {
        Self {
            bracket_id: bracket.id,
            player_id: player.id
        }
    }
    save_fn!(player_bracket_entry::table, PlayerBracketEntry);
}