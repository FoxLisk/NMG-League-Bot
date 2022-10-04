use diesel::prelude::Insertable;
use rocket::serde::json::serde_json;
use crate::models::brackets::Bracket;
use crate::models::epoch_timestamp;
use crate::models::player::Player;
use crate::models::race::Race;
use crate::schema::bracket_races;

#[derive(serde::Serialize, serde::Deserialize)]
enum BracketRaceState {
    New,
    Scheduled,
    Finished
}

#[derive(Queryable)]
struct BracketRace {
    id: i32,
    bracket_id: i32,
    player_1_id: i32,
    player_2_id: i32,
    async_race_id: Option<i32>,
    scheduled_for: Option<i64>,
    state: String,
    player_1_result: Option<String>,
    player_2_result: Option<String>,
    outcome: Option<String>,
}

#[derive(Insertable)]
#[diesel(table_name=bracket_races)]
struct NewBracketRace {
    bracket_id: i32,
    player_1_id: i32,
    player_2_id: i32,
    async_race_id: Option<i32>,
    scheduled_for: Option<i64>,
    state: String,
    player_1_result: Option<String>,
    player_2_result: Option<String>,
    outcome: Option<String>,
}

impl NewBracketRace {
    // i think we probably always create these without anything scheduled
    fn new(bracket: &Bracket, player_1: &Player, player_2: &Player) -> Self {
        Self {
            bracket_id: bracket.id,
            player_1_id: player_1.id,
            player_2_id: player_2.id,
            async_race_id: None,
            scheduled_for: None,
            state: serde_json::to_string(&BracketRaceState::New).unwrap_or("Unknown".to_string()),
            player_1_result: None,
            player_2_result: None,
            outcome: None
        }
    }
}
