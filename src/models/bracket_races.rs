use crate::models::bracket_race_infos::BracketRaceInfo;
use crate::models::bracket_rounds::BracketRound;
use crate::models::brackets::Bracket;
use crate::models::player::Player;
use crate::models::season::Season;
use crate::save_fn;
use crate::schema::bracket_races;
use crate::update_fn;
use crate::utils::format_hms;
use crate::BracketRaceState;
use crate::BracketRaceStateError;
use crate::NMGLeagueBotError;
use chrono::{DateTime, Duration, TimeZone};
use diesel::prelude::*;
use diesel::SqliteConnection;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use swiss_pairings::MatchResult;

#[derive(serde::Serialize, serde::Deserialize)]
pub enum PlayerResult {
    Forfeit,
    /// finish time in seconds
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
    pub fn time(&self) -> u32 {
        match self {
            Self::Forfeit => Duration::hours(3).num_seconds() as u32,
            Self::Finish(t) => t.clone(),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub enum Outcome {
    Tie,
    P1Win,
    P2Win,
}

#[derive(Queryable, Identifiable, AsChangeset, Debug, Serialize, Clone, Selectable)]
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

    pub fn unscheduled(conn: &mut SqliteConnection) -> Result<Vec<Self>, NMGLeagueBotError> {
        let state = serde_json::to_string(&BracketRaceState::New)?;
        bracket_races::table
            .filter(bracket_races::state.eq(state))
            .load(conn)
            .map_err(From::from)
    }

    pub fn scheduled(conn: &mut SqliteConnection) -> Result<Vec<Self>, NMGLeagueBotError> {
        let state = serde_json::to_string(&BracketRaceState::Scheduled)?;
        bracket_races::table
            .filter(bracket_races::state.eq(state))
            .load(conn)
            .map_err(From::from)
    }

    pub fn get_unfinished_races_for_player(
        player: &Player,
        conn: &mut SqliteConnection,
    ) -> Result<Vec<BracketRace>, NMGLeagueBotError> {
        bracket_races::table
            .filter(bracket_races::state.ne(serde_json::to_string(&BracketRaceState::Finished)?))
            .filter(
                bracket_races::player_1_id
                    .eq(player.id)
                    .or(bracket_races::player_2_id.eq(player.id)),
            )
            .load(conn)
            .map_err(From::from)
    }
}

impl BracketRace {
    pub fn state(&self) -> Result<BracketRaceState, serde_json::Error> {
        serde_json::from_str(&self.state)
    }

    fn set_state(&mut self, state: BracketRaceState) {
        self.state = serde_json::to_string(&state).unwrap_or("Unknown".to_string());
    }

    /// Returns the [Outcome] of the race if it is complete, None if it's not complete, and an error if
    /// this model has invalid data.
    pub fn outcome(&self) -> Result<Option<Outcome>, serde_json::Error> {
        match self.outcome.as_ref() {
            None => Ok(None),
            Some(o) => Ok(Some(serde_json::from_str(o)?)),
        }
    }

    /// Returns true if the race is definitely complete, false if it is incomplete or there is a data error.
    pub fn is_complete(&self) -> bool {
        match self.outcome() {
            Ok(Some(_)) => true,
            _ => false,
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
        let mut players = crate::schema::players::table
            .filter(crate::schema::players::id.eq_any(vec![self.player_1_id, self.player_2_id]))
            .load::<Player>(conn)?
            .into_iter()
            .map(|p| (p.id, p))
            .collect::<HashMap<_, _>>();
        let p1 = players
            .remove(&self.player_1_id)
            .ok_or(diesel::result::Error::NotFound)?;

        let p2 = players
            .remove(&self.player_2_id)
            .ok_or(diesel::result::Error::NotFound)?;
        Ok((p1, p2))
    }

    pub fn involves_player(&self, player: &Player) -> bool {
        self.player_1_id == player.id || self.player_2_id == player.id
    }

    pub fn bracket(&self, conn: &mut SqliteConnection) -> Result<Bracket, diesel::result::Error> {
        Bracket::get_by_id(self.bracket_id, conn)
    }

    /// this hits the db (twice!) to find players, so uh. i guess if that matters to you don't call it
    /// has users names instead of mentions, because mentions don't work in embeds
    pub fn title(&self, conn: &mut SqliteConnection) -> Result<String, diesel::result::Error> {
        let (p1, p2) = self.players(conn)?;
        Ok(format!("{} vs {}", p1.name, p2.name))
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
                return Err(BracketRaceStateError::InvalidState(
                    vec![BracketRaceState::New, BracketRaceState::Scheduled],
                    BracketRaceState::Finished,
                ));
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

    /// only works on runs in the New or Scheduled state unless force is true  
    /// will overwrite existing partial results with new results but won't set them to null  
    /// updates state & outcome to finished if that is the case  
    pub fn add_results(
        &mut self,
        p1: Option<&PlayerResult>,
        p2: Option<&PlayerResult>,
        force: bool,
    ) -> Result<(), BracketRaceStateError> {
        let state = self.state()?;
        if !force && state == BracketRaceState::Finished {
            return Err(BracketRaceStateError::InvalidState(
                vec![BracketRaceState::Finished],
                state,
            ));
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

    pub fn player_1_result(&self) -> Option<Result<PlayerResult, serde_json::Error>> {
        self.player_1_result
            .as_ref()
            // not sure why `.map(serde_json::from_str)` doesnt work here but this does. it complains
            // about &String vs &str in the former invocation
            .map(|s| serde_json::from_str(s))
    }

    pub fn player_2_result(&self) -> Option<Result<PlayerResult, serde_json::Error>> {
        self.player_2_result
            .as_ref()
            .map(|s| serde_json::from_str(s))
    }

    fn finish(&mut self) -> Result<(), BracketRaceStateError> {
        let (p1, p2) = match (self.player_1_result(), self.player_2_result()) {
            (Some(p1r), Some(p2r)) => (p1r?, p2r?),
            _ => {
                return Err(BracketRaceStateError::MissingResult);
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

#[derive(Insertable, Debug)]
#[diesel(table_name=bracket_races)]
pub struct NewBracketRace {
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

    save_fn!(bracket_races::table, BracketRace);
}
