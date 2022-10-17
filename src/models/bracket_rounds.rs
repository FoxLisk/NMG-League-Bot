use crate::models::bracket_races::{BracketRace, BracketRaceState};
use crate::models::brackets::Bracket;
use crate::save_fn;
use crate::schema::bracket_rounds;
use diesel::prelude::*;

#[derive(Queryable)]
pub struct BracketRound {
    pub id: i32,
    pub round_num: i32,
    bracket_id: i32,
}

impl BracketRound {
    pub fn races(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<Vec<BracketRace>, diesel::result::Error> {
        use crate::schema::bracket_races;
        bracket_races::table
            .filter(bracket_races::round_id.eq(self.id))
            .load(conn)
    }

    pub fn all_races_finished(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<bool, diesel::result::Error> {
        use crate::schema::bracket_races;
        // TODO: care about infallible serialization?
        let s = serde_json::to_string(&BracketRaceState::Finished).unwrap();
        bracket_races::table
            .filter(bracket_races::round_id.eq(self.id))
            .filter(bracket_races::state.ne(&s))
            .load(conn)
            .map(|v: Vec<BracketRace>| v.is_empty())
    }
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
            bracket_id: bracket.id,
        }
    }

    save_fn!(bracket_rounds::table, BracketRound);
}
