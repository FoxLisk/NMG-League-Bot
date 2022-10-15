use crate::models::bracket_rounds::BracketRound;
use crate::models::brackets::Bracket;
use crate::models::brackets::BracketError::InvalidState;
use crate::models::epoch_timestamp;
use crate::models::player::Player;
use crate::models::race::Race;
use crate::models::season::Season;
use crate::schema::bracket_races;
use crate::update_fn;
use diesel::prelude::*;
use diesel::SqliteConnection;
use rocket::serde::json::serde_json;
use serde_json::Error;
use swiss_pairings::MatchResult;

#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq, Debug)]
pub enum BracketRaceState {
    New,
    Scheduled,
    Finished,
}

#[derive(Debug)]
pub enum BracketRaceStateError {
    InvalidState,
    ParseError(serde_json::Error),
}

impl From<serde_json::Error> for BracketRaceStateError {
    fn from(e: Error) -> Self {
        Self::ParseError(e)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum PlayerResult {
    Forfeit,
    Finish(u32),
}

#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub enum Outcome {
    Tie,
    P1Win,
    P2Win,
}

#[derive(Queryable, Identifiable, AsChangeset, Debug)]
pub struct BracketRace {
    id: i32,
    bracket_id: i32,
    round_id: i32,
    pub player_1_id: i32,
    pub player_2_id: i32,
    async_race_id: Option<i32>,
    scheduled_for: Option<i64>,
    state: String,
    player_1_result: Option<String>,
    player_2_result: Option<String>,
    outcome: Option<String>,
}

#[derive(Debug)]
pub enum MatchResultError {
    InvalidOutcome,
    RaceNotFinished,
}

impl BracketRace {

    pub fn try_into_match_result<'a>(&'a self) -> Result<MatchResult<'a, i32>, MatchResultError> {
        if self
            .state()
            .map_err(|_| MatchResultError::RaceNotFinished)?
            != BracketRaceState::Finished
        {
            return Err(MatchResultError::RaceNotFinished);
        }
        let o = self
            .outcome()
            .map_err(|_| MatchResultError::InvalidOutcome)?
            .ok_or(MatchResultError::RaceNotFinished)?;
        Ok(match o {
            Outcome::Tie => MatchResult::Draw {
                p1: &self.player_1_id,
                p2: &self.player_2_id,
            },
            Outcome::P1Win => MatchResult::Player1Win {
                p1: &self.player_1_id,
                p2: &self.player_2_id,
            },

            Outcome::P2Win => MatchResult::Player2Win {
                p1: &self.player_1_id,
                p2: &self.player_2_id,
            },
        })
    }
}

impl BracketRace {
    pub fn state(&self) -> Result<BracketRaceState, BracketRaceStateError> {
        serde_json::from_str(&self.state).map_err(From::from)
    }

    fn set_state(&mut self, state: BracketRaceState) {
        self.state = serde_json::to_string(&state).unwrap_or("Unknown".to_string());
    }

    pub fn outcome(&self) -> Result<Option<Outcome>, serde_json::Error> {
        match self.outcome.as_ref() {
            None => Ok(None),
            Some(o) => Ok(Some(serde_json::from_str(o)?)),
        }
    }

    /// only works on runs in the New or Scheduled state
    /// will overwrite existing partial results with new results but won't set them to null
    /// updates state & outcome to finished if that is the case
    pub fn add_results(
        &mut self,
        p1: Option<PlayerResult>,
        p2: Option<PlayerResult>,
    ) -> Result<(), BracketRaceStateError> {
        if self.state()? == BracketRaceState::Finished {
            return Err(BracketRaceStateError::InvalidState);
        }
        if let Some(p1r) = p1 {
            self.player_1_result = Some(serde_json::to_string(&p1r)?);
        }
        if let Some(p2r) = p2 {
            self.player_2_result = Some(serde_json::to_string(&p2r)?);
        }
        if self.player_1_result.is_some() && self.player_2_result.is_some() {
            self.finish()?;
        }
        Ok(())
    }

    fn player_1_result(&self) -> Option<PlayerResult> {
        self.player_1_result
            .as_ref()
            .and_then(|s| serde_json::from_str(s).ok())
    }

    fn player_2_result(&self) -> Option<PlayerResult> {
        self.player_2_result
            .as_ref()
            .and_then(|s| serde_json::from_str(s).ok())
    }

    fn finish(&mut self) -> Result<(), BracketRaceStateError> {
        let (p1, p2) = match (self.player_1_result(), self.player_2_result()) {
            (Some(p1r), Some(p2r)) => (p1r, p2r),
            _ => {
                return Err(BracketRaceStateError::InvalidState);
            }
        };

        let outcome = match (p1, p2) {
            (PlayerResult::Forfeit, PlayerResult::Forfeit) => Outcome::Tie,
            (PlayerResult::Forfeit, _) => Outcome::P2Win,
            (_, PlayerResult::Forfeit) => Outcome::P1Win,
            (PlayerResult::Finish(p1t), PlayerResult::Finish(p2t)) => {
                if p1t < p2t {
                    Outcome::P1Win
                } else if p1t > p2t {
                    Outcome::P2Win
                } else {
                    Outcome::Tie
                }
            }
        };
        self.outcome = Some(serde_json::to_string(&outcome)?);
        self.set_state(BracketRaceState::Finished);
        Ok(())
    }

    update_fn! {}
}

pub fn get_current_round_race_for_player(
    player: &Player,
    conn: &mut SqliteConnection,
) -> Result<Option<BracketRace>, diesel::result::Error> {
    let sn = match Season::get_active_season(conn)? {
        Some(s) => s,
        None => {
            return Ok(None);
        }
    };

    for bracket in sn.brackets(conn)? {
        let round = match bracket.current_round(conn)? {
            Some(r) => r,
            None => {
                continue;
            }
        };
        let mut races: Vec<BracketRace> = bracket_races::table
            .filter(bracket_races::round_id.eq(round.id))
            .filter(
                bracket_races::player_1_id
                    .eq(player.id)
                    .or(bracket_races::player_2_id.eq(player.id)),
            )
            .load(conn)?;
        if races.is_empty() {
            continue;
        } else {
            if races.len() != 1 {
                println!("Multiple races for same racer?");
            }
            return Ok(races.pop());
        }
    }

    Ok(None)
}

#[derive(Insertable)]
#[diesel(table_name=bracket_races)]
pub struct NewBracketRace {
    bracket_id: i32,
    round_id: i32,
    pub player_1_id: i32,
    pub player_2_id: i32,
    async_race_id: Option<i32>,
    scheduled_for: Option<i64>,
    state: String,
    player_1_result: Option<String>,
    player_2_result: Option<String>,
    outcome: Option<String>,
}

pub fn insert_bulk(new_races: &Vec<NewBracketRace>, conn: &mut SqliteConnection) -> Result<usize, diesel::result::Error> {
    diesel::insert_into(bracket_races::table)
        .values(new_races)
        .execute(conn)
}

impl NewBracketRace {
    // i think we probably always create these without anything scheduled
    pub fn new(
        bracket: &Bracket,
        round: &BracketRound,
        player_1: &Player,
        player_2: &Player,
    ) -> Self {
        Self {
            bracket_id: bracket.id,
            round_id: round.id,
            player_1_id: player_1.id,
            player_2_id: player_2.id,
            async_race_id: None,
            scheduled_for: None,
            state: serde_json::to_string(&BracketRaceState::New).unwrap_or("Unknown".to_string()),
            player_1_result: None,
            player_2_result: None,
            outcome: None,
        }
    }
}
