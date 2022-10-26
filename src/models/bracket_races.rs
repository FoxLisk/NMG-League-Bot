use crate::models::bracket_race_infos::BracketRaceInfo;
use crate::models::bracket_rounds::BracketRound;
use crate::models::brackets::Bracket;
use crate::models::player::Player;
use crate::models::season::Season;
use crate::schema::bracket_races;
use crate::update_fn;
use crate::utils::format_hms;
use chrono::{DateTime, TimeZone};
use diesel::prelude::*;
use diesel::SqliteConnection;
use serde::Serialize;
use std::fmt::{Display, Formatter};
use swiss_pairings::MatchResult;
use thiserror::Error;

#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq, Debug)]
pub enum BracketRaceState {
    New,
    Scheduled,
    Finished,
}

#[derive(Debug, Error)]
pub enum BracketRaceStateError {
    #[error("Invalid state")]
    InvalidState,
    #[error("Deserialization error: {0}")]
    ParseError(#[from] serde_json::Error),
    #[error("Database error: {0}")]
    DatabaseError(#[from] diesel::result::Error),
}


#[derive(serde::Serialize, serde::Deserialize)]
pub enum PlayerResult {
    Forfeit,
    Finish(u32),
}

impl Display for PlayerResult {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayerResult::Forfeit => {
                write!(f, "Forfeit")
            }
            PlayerResult::Finish(t) => {
                write!(f, "{}", format_hms(*t as u64))
            }
        }
    }
}

impl PlayerResult {
    /// finish time if given, 3:00:00 if forfeit
    pub  fn time(&self) -> u32 {
        match self {
            Self::Forfeit => 60 * 60 * 180,
            Self::Finish(t) => t.clone()
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub enum Outcome {
    Tie,
    P1Win,
    P2Win,
}


#[derive(Queryable, Identifiable, AsChangeset, Debug, Serialize, Clone)]
pub struct BracketRace {
    pub id: i32,
    pub bracket_id: i32,
    pub round_id: i32,
    pub player_1_id: i32,
    pub player_2_id: i32,
    pub async_race_id: Option<i32>,
    pub state: String,
    pub player_1_result: Option<String>,
    pub player_2_result: Option<String>,
    pub outcome: Option<String>,
}

impl BracketRace {}

#[derive(Debug)]
pub enum MatchResultError {
    InvalidOutcome,
    RaceNotFinished,
}

impl BracketRace {
    /// this expects the object to exist, so it returns Self instead of Option<Self>
    pub fn get_by_id(id: i32, conn: &mut SqliteConnection) -> Result<Self, diesel::result::Error> {
        bracket_races::table.find(id).first(conn)
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

    pub fn info(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<BracketRaceInfo, diesel::result::Error> {
        BracketRaceInfo::get_or_create_for_bracket(self, conn)
    }

    /// returns (Player 1, Player 2), specifically in order
    pub fn players(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<(Player, Player), diesel::result::Error> {
        // TODO multiple queries bad 😫
        let p1 =
            Player::get_by_id(self.player_1_id, conn)?.ok_or(diesel::result::Error::NotFound)?;
        let p2 =
            Player::get_by_id(self.player_2_id, conn)?.ok_or(diesel::result::Error::NotFound)?;
        Ok((p1, p2))
    }

    pub fn bracket(&self, conn: &mut SqliteConnection) -> Result<Bracket, diesel::result::Error> {
        Bracket::get_by_id(self.bracket_id, conn)
    }

    /// this hits the db (twice!) to find players, so uh. i guess if that matters to you don't call it
    pub fn title(&self, conn: &mut SqliteConnection) -> Result<String, diesel::result::Error> {
        let (p1, p2) = self.players(conn)?;
        Ok(format!(
            "{} vs {}",
            p1.mention_or_name(),
            p2.mention_or_name()
        ))
    }

    /// returns (old_info, new_info) (before and after the update from this method
    /// updates the database
    pub fn schedule<T: TimeZone>(
        &mut self,
        when: &DateTime<T>,
        conn: &mut SqliteConnection,
    ) -> Result<(BracketRaceInfo, BracketRaceInfo), BracketRaceStateError> {
        match self.state()? {
            BracketRaceState::New | BracketRaceState::Scheduled => {}
            BracketRaceState::Finished => {
                return Err(BracketRaceStateError::InvalidState);
            }
        };
        let mut info = self.info(conn)?;
        let prior = info.clone();
        info.schedule(when, conn)?;
        self.set_state(BracketRaceState::Scheduled);
        conn.transaction(|c| {
            info.update(c)?;
            self.update(c)
        })?;
        Ok((prior, info))
    }

    /// only works on runs in the New or Scheduled state
    /// will overwrite existing partial results with new results but won't set them to null
    /// updates state & outcome to finished if that is the case
    pub fn add_results(
        &mut self,
        p1: Option<&PlayerResult>,
        p2: Option<&PlayerResult>,
    ) -> Result<(), BracketRaceStateError> {
        if self.state()? == BracketRaceState::Finished {
            return Err(BracketRaceStateError::InvalidState);
        }
        if let Some(p1r) = p1 {
            self.player_1_result = Some(serde_json::to_string(p1r)?);
        }
        if let Some(p2r) = p2 {
            self.player_2_result = Some(serde_json::to_string(p2r)?);
        }
        if self.player_1_result.is_some() && self.player_2_result.is_some() {
            self.finish()?;
        }
        Ok(())
    }

    pub fn player_1_result(&self) -> Option<PlayerResult> {
        self.player_1_result
            .as_ref()
            .and_then(|s| serde_json::from_str(s).ok())
    }


    pub fn player_2_result(&self) -> Option<PlayerResult> {
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

    pub fn try_into_match_result(&self) -> Result<MatchResult<i32>, MatchResultError> {
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
    state: String,
    player_1_result: Option<String>,
    player_2_result: Option<String>,
    outcome: Option<String>,
}

pub fn insert_bulk(
    new_races: &Vec<NewBracketRace>,
    conn: &mut SqliteConnection,
) -> Result<usize, diesel::result::Error> {
    diesel::insert_into(bracket_races::table)
        .values(new_races)
        .execute(conn)
}

impl NewBracketRace {
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
            state: serde_json::to_string(&BracketRaceState::New).unwrap_or("Unknown".to_string()),
            player_1_result: None,
            player_2_result: None,
            outcome: None,
        }
    }
}
