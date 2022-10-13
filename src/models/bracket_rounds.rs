use diesel::prelude::*;
use crate::models::brackets::Bracket;
use crate::models::epoch_timestamp;
use crate::models::player::Player;
use crate::models::race::Race;
use crate::schema::bracket_rounds;
use crate::save_fn;

#[derive(Queryable)]
pub struct BracketRound {
    pub id: i32,
    round_num: i32,
    bracket_id: i32,
}

#[derive(Insertable)]
#[diesel(table_name=bracket_rounds)]
pub struct NewBracketRound {
    round_num: i32,
    bracket_id: i32,
}

impl NewBracketRound {
    pub fn new(bracket: &Bracket, round_num: i32) -> Self {
        Self {
            round_num,
            bracket_id: bracket.id
        }
    }


    save_fn!(bracket_rounds::table, BracketRound);
}