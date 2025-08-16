use crate::models::bracket_races::{
    insert_bulk, BracketRace, MatchResultError, NewBracketRace, Outcome,
};
use crate::models::bracket_rounds::{BracketRound, NewBracketRound};
use crate::models::player::Player;
use crate::models::season::Season;
use crate::schema::brackets;
use crate::{save_fn, update_fn, BracketRaceStateError, NMGLeagueBotError};
use diesel::prelude::*;
use diesel::result::Error;
use diesel::{RunQueryDsl, SqliteConnection};
use enum_iterator::Sequence;
use itertools::Itertools;
use log::{debug, warn};
use rand::thread_rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use swiss_pairings::{PairingError, TourneyConfig};
use thiserror::Error;

use rand::seq::SliceRandom;

use super::bracket_races::PlayerResult;

#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq, Debug)]
pub enum BracketState {
    Unstarted,
    Started,
    Finished,
}

#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq, Debug, Sequence)]
pub enum BracketType {
    Swiss,
    RoundRobin,
}

#[derive(Queryable, Identifiable, Debug, AsChangeset, Serialize, Deserialize, Selectable)]
#[allow(unused)]
pub struct Bracket {
    pub id: i32,
    pub name: String,
    pub season_id: i32,
    state: String,
    bracket_type: String,
    /// set for backfilled brackets to give a little context on the bracket pages
    pub backfill_note: Option<String>,
}

impl Bracket {}

#[derive(Debug, Error)]
pub enum BracketError {
    #[error("Invalid bracket state")]
    InvalidState,
    #[error("Cannot generate pairings with odd player counts. Add a bye?")]
    OddPlayerCount,
    #[error("Database error: {0}")]
    DBError(#[from] diesel::result::Error),
    #[error("Uncategorized error: {0}")]
    Other(String),
    #[error("Bracket Race state error: {0}")]
    BracketRaceStateError(#[from] BracketRaceStateError),
    #[error("Serialization error (probably from invalid db state): {0}")]
    SerializationError(#[from] serde_json::Error),
    #[error("Match result error: {0:?}")]
    MatchResultError(MatchResultError),
    #[error("Pairings error: {0:?}")]
    PairingError(PairingError),
    #[error("Round robin error: {0}")]
    RoundRobinError(String),
}
impl From<PairingError> for BracketError {
    fn from(e: PairingError) -> Self {
        Self::PairingError(e)
    }
}
impl From<MatchResultError> for BracketError {
    fn from(e: MatchResultError) -> Self {
        Self::MatchResultError(e)
    }
}
impl From<String> for BracketError {
    fn from(e: String) -> Self {
        Self::Other(e)
    }
}

fn generate_next_round_pairings_swiss(
    bracket: &Bracket,
    conn: &mut SqliteConnection,
) -> Result<(), BracketError> {
    let rounds = bracket.rounds(conn)?;

    let mut round_races = vec![];

    let mut highest_round_num = 0;
    for round in rounds {
        round_races.push(round.races(conn)?);
        assert!(round.round_num > highest_round_num);
        highest_round_num = round.round_num;
    }
    let mut pairing_rounds = vec![];
    for races in &round_races {
        let mut this_round = vec![];
        for race in races {
            this_round.push(race.try_into_match_result()?);
        }
        pairing_rounds.push(this_round);
    }
    debug!("{:?}", pairing_rounds);
    let cfg = TourneyConfig {
        points_per_win: 2,
        points_per_loss: 0,
        points_per_draw: 1,
        error_on_repeated_opponent: true,
    };
    let (pairings, _standings) =
        swiss_pairings::swiss_pairings(&pairing_rounds, &cfg, Some(Duration::from_millis(5000)))?;
    debug!("{:?}", pairings);

    let mut players: HashMap<_, _> =
        HashMap::from_iter(bracket.players(conn)?.into_iter().map(|p| (p.id, p)));

    let nr = NewBracketRound::new(&bracket, highest_round_num + 1);
    let new_round = nr.save(conn)?;
    let mut new_races = vec![];
    for (p1_id, p2_id) in pairings {
        let p1 = players
            .remove(p1_id)
            .ok_or(BracketError::Other(format!("Cannot find player {}", p1_id)))?;
        let p2 = players
            .remove(p2_id)
            .ok_or(BracketError::Other(format!("Cannot find player {}", p2_id)))?;
        let new_race = NewBracketRace::new(bracket, &new_round, &p1, &p2);
        new_races.push(new_race);
    }
    insert_bulk(&new_races, conn)?;

    Ok(())
}

fn generate_next_round_pairings(
    bracket: &Bracket,
    conn: &mut SqliteConnection,
) -> Result<(), BracketError> {
    if bracket.state()? != BracketState::Started {
        return Err(BracketError::InvalidState);
    }
    match bracket.bracket_type()? {
        BracketType::Swiss => generate_next_round_pairings_swiss(bracket, conn),
        BracketType::RoundRobin => Err(BracketError::RoundRobinError(
            "Round Robin pairings already generated".to_string(),
        )),
    }
}

fn generate_initial_pairings_swiss(
    bracket: &mut Bracket,
    conn: &mut SqliteConnection,
) -> Result<(), BracketError> {
    let new_round = NewBracketRound::new(bracket, 1);
    let round = new_round.save(conn)?;

    let mut players = bracket.players(conn)?;
    players.as_mut_slice().shuffle(&mut thread_rng());
    let mut nbrs = vec![];
    while players.len() > 1 {
        let p1 = players.pop().unwrap();
        let p2 = players.pop().unwrap();
        let nbr = NewBracketRace::new(bracket, &round, &p1, &p2);
        nbrs.push(nbr);
    }
    if !players.is_empty() {
        return Err(BracketError::OddPlayerCount);
    }
    insert_bulk(&nbrs, conn)?;

    bracket
        .set_state(BracketState::Started)
        .map_err(|e| e.to_string())?;
    bracket.update(conn)?;
    Ok(())
}

fn generate_pairings_round_robin(
    bracket: &Bracket,
    round: &BracketRound,
    conn: &mut SqliteConnection,
) -> Result<(), BracketError> {
    let mut players = bracket.players(conn)?;
    players.sort_unstable_by_key(|p| p.id);

    fn sorted_tuple<'a>(ps: (&'a Player, &'a Player)) -> (&'a Player, &'a Player) {
        let (p1, p2) = ps;
        if p1.id < p2.id {
            (p1, p2)
        } else {
            (p2, p1)
        }
    }

    let product = itertools::iproduct!(players.iter(), players.iter()).collect::<Vec<_>>();
    let filtered = product
        .iter()
        .filter(|(p1, p2)| p1.id != p2.id)
        .collect::<Vec<_>>();
    let uniq = filtered.iter().unique().collect::<Vec<_>>();
    warn!(
        "Product (count): {}, filtered: {filtered:?}, uniq: {uniq:?}",
        product.len()
    );

    let nbrs: Vec<_> = itertools::iproduct!(players.iter(), players.iter())
        .filter(|(p1, p2)| p1.id != p2.id)
        .map(sorted_tuple)
        .unique()
        .map(|(p1, p2)| NewBracketRace::new(bracket, round, p1, p2))
        .collect::<_>();
    let expected = (players.len() * (players.len() - 1)) / 2;
    if nbrs.len() != expected {
        return Err(BracketError::Other(format!(
            "Expected {expected} pairings, got {}",
            nbrs.len()
        )));
    }
    debug!("NBRs: {nbrs:?}");
    insert_bulk(&nbrs, conn)?;
    Ok(())
}

fn generate_initial_pairings_round_robin(
    bracket: &mut Bracket,
    conn: &mut SqliteConnection,
) -> Result<(), BracketError> {
    // hmm maybe we have to keep this? i think we have some assumptions about the concept of
    // "round" which maybe should be rethought (but not today)
    let new_round = NewBracketRound::new(bracket, 1);
    let round = new_round.save(conn)?;
    // TODO: inline this function?
    generate_pairings_round_robin(bracket, &round, conn)?;

    // TODO: setting state can be pulled up an abstraction layer
    bracket
        .set_state(BracketState::Started)
        .map_err(|e| e.to_string())?;

    bracket.update(conn)?;
    Ok(())
}

fn generate_initial_pairings(
    bracket: &mut Bracket,
    conn: &mut SqliteConnection,
) -> Result<(), BracketError> {
    if bracket.state()? != BracketState::Unstarted {
        return Err(BracketError::InvalidState);
    }
    if let Some(_) = bracket.current_round(conn)? {
        debug!("Bracket round already exists for this bracket in 'unstarted' state?!");
        return Err(BracketError::InvalidState);
    }
    match bracket.bracket_type()? {
        BracketType::Swiss => generate_initial_pairings_swiss(bracket, conn),
        BracketType::RoundRobin => generate_initial_pairings_round_robin(bracket, conn),
    }
}

impl Bracket {
    pub fn state(&self) -> Result<BracketState, serde_json::Error> {
        serde_json::from_str(&self.state)
    }

    pub fn is_unstarted(&self) -> Result<bool, serde_json::Error> {
        Ok(self.state()? == BracketState::Unstarted)
    }

    pub fn is_finished(&self) -> Result<bool, serde_json::Error> {
        Ok(self.state()? == BracketState::Finished)
    }

    pub fn bracket_type(&self) -> Result<BracketType, serde_json::Error> {
        serde_json::from_str(&self.bracket_type)
    }

    fn set_state(&mut self, state: BracketState) -> Result<(), serde_json::Error> {
        self.state = serde_json::to_string(&state)?;
        Ok(())
    }

    /// sets this bracket's state to finished, if there are no unfinished rounds
    pub fn finish(&mut self, cxn: &mut SqliteConnection) -> Result<bool, NMGLeagueBotError> {
        for r in self.rounds(cxn)? {
            if !r.all_races_finished(cxn)? {
                return Ok(false);
            }
        }
        self.set_state(BracketState::Finished)?;
        Ok(true)
    }

    update_fn! {}

    pub fn players(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<Vec<Player>, diesel::result::Error> {
        use crate::schema::player_bracket_entry as pbes;
        use crate::schema::players;
        pbes::table
            .filter(pbes::bracket_id.eq(self.id))
            .inner_join(players::table)
            .select(players::all_columns)
            .load(conn)
    }

    pub fn rounds(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<Vec<BracketRound>, diesel::result::Error> {
        use crate::schema::bracket_rounds;
        bracket_rounds::table
            .filter(bracket_rounds::bracket_id.eq(self.id))
            .order(bracket_rounds::round_num.asc())
            .load(conn)
    }

    /// returns all BracketRaces for this bracket (unordered)
    pub fn bracket_races(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<Vec<BracketRace>, diesel::result::Error> {
        use crate::schema::bracket_races;
        bracket_races::table
            .filter(bracket_races::bracket_id.eq(self.id))
            .load(conn)
    }

    pub fn get_by_id(id: i32, conn: &mut SqliteConnection) -> Result<Bracket, Error> {
        brackets::table.find(id).first(conn)
    }

    /// creates and saves a bunch of BracketRaces representing the next round's matchups
    pub fn generate_pairings(&mut self, conn: &mut SqliteConnection) -> Result<(), BracketError> {
        let state = self.state().map_err(|_| BracketError::InvalidState)?;
        match state {
            BracketState::Unstarted => conn.transaction(|c| generate_initial_pairings(self, c)),
            BracketState::Started => conn.transaction(|c| generate_next_round_pairings(self, c)),
            BracketState::Finished => Err(BracketError::InvalidState),
        }
    }

    pub fn current_round(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<Option<BracketRound>, diesel::result::Error> {
        use crate::schema::bracket_rounds;
        let mut brs: Vec<BracketRound> = bracket_rounds::table
            .filter(bracket_rounds::bracket_id.eq(self.id))
            .order(bracket_rounds::round_num.desc())
            .limit(1)
            .load(conn)?;
        Ok(brs.pop())
    }

    pub fn standings(&self, conn: &mut SqliteConnection) -> Result<Vec<PlayerInfo>, BracketError> {
        match self.state()? {
            BracketState::Unstarted => {
                return Err(BracketError::InvalidState);
            }
            BracketState::Started | BracketState::Finished => {}
        }
        let rounds = self.rounds(conn)?;
        let mut races = vec![];
        let is_rr = match self.bracket_type()? {
            BracketType::Swiss => false,
            BracketType::RoundRobin => true,
        };

        struct StandingsRace {
            player_1_id: i32,
            player_2_id: i32,
            player_1_result: PlayerResult,
            player_2_result: PlayerResult,
            outcome: Outcome,
        }

        impl TryFrom<BracketRace> for StandingsRace {
            type Error = BracketError;

            fn try_from(value: BracketRace) -> Result<Self, Self::Error> {
                let player_1_result = value
                    .player_1_result()
                    .ok_or(BracketError::InvalidState)??;
                let player_2_result = value
                    .player_2_result()
                    .ok_or(BracketError::InvalidState)??;
                let outcome = value.outcome()?.ok_or(BracketError::InvalidState)?;
                Ok(Self {
                    player_1_id: value.player_1_id,
                    player_2_id: value.player_2_id,
                    player_1_result,
                    player_2_result,
                    outcome,
                })
            }
        }

        for round in rounds {
            let round_races = round.races(conn)?;
            if !is_rr && !round_races.iter().all(BracketRace::is_complete) {
                // we don't want to show standings mid-round for swiss brackets because they're very ugly IMO
                // (having the brackets be like 1. 2-1 guy, 2. 2-0 guy looks awful)
                // but for RR brackets I think it's fine; they're a little messier, but it sucks to not show
                // any results until the very end.
                break;
            }

            races.extend(
                round_races
                    .into_iter()
                    .filter_map(|r| StandingsRace::try_from(r).ok()),
            );
        }

        let mut info: HashMap<i32, PlayerInfoBuilder> = Default::default();
        for race in races {
            let StandingsRace {
                player_1_id,
                player_2_id,
                player_1_result,
                player_2_result,
                outcome,
            } = race;
            let (p1_adjust, p2_adjust) = match outcome {
                Outcome::Tie => (1, 1),
                Outcome::P1Win => (2, 0),
                Outcome::P2Win => (0, 2),
            };
            let p1_i_b = info
                .entry(race.player_1_id)
                .or_insert(PlayerInfoBuilder::new(race.player_1_id));
            p1_i_b.results.push(player_1_result);
            p1_i_b.points += p1_adjust;
            p1_i_b.opponents.push(player_2_id);

            let p2_i_b = info
                .entry(race.player_2_id)
                .or_insert(PlayerInfoBuilder::new(race.player_2_id));
            p2_i_b.results.push(player_2_result);
            p2_i_b.points += p2_adjust;
            p2_i_b.opponents.push(player_1_id);
        }
        let points: HashMap<i32, i32> = info.values().map(|p| (p.id, p.points)).collect();

        Ok(info
            .into_values()
            .map(|builder| builder.build(&points))
            .sorted_by_cached_key(|p| (-p.points, -p.opponent_points, p.time_adjusted(), p.id))
            .collect())
    }
}

struct PlayerInfoBuilder {
    id: i32,
    points: i32,
    opponents: Vec<i32>,
    results: Vec<PlayerResult>,
}

impl PlayerInfoBuilder {
    fn new(id: i32) -> Self {
        Self {
            id,
            points: 0,
            opponents: vec![],
            results: vec![],
        }
    }

    fn build(self, scores: &HashMap<i32, i32>) -> PlayerInfo {
        let score = self
            .opponents
            .iter()
            .map(|opponent_id| {
                if let Some(score) = scores.get(opponent_id) {
                    score.clone()
                } else {
                    warn!("PlayerInfoBuilder unable to find score for player {opponent_id}");
                    0
                }
            })
            .sum();

        PlayerInfo {
            id: self.id,
            points: self.points,
            opponent_points: score,
            results: self.results,
        }
    }
}

pub struct PlayerInfo {
    pub id: i32,
    /// this is really points*2, convert to float elsewhere
    pub points: i32,
    /// see [points]
    pub opponent_points: i32,
    results: Vec<PlayerResult>,
}

impl PlayerInfo {
    /// total time of all races, with forfeits counting as 3 hours (for use in sorting)
    fn time_adjusted(&self) -> u32 {
        self.results.iter().map(|r| r.time()).sum()
    }

    pub fn avg_time_adjusted(&self) -> f32 {
        self.time_adjusted() as f32 / self.results.len() as f32
    }

    pub fn avg_time_finished(&self) -> f32 {
        let finished = self
            .results
            .iter()
            .filter_map(|r| match r {
                PlayerResult::Forfeit => None,
                PlayerResult::Finish(t) => Some(*t),
            })
            .collect::<Vec<_>>();
        if finished.is_empty() {
            return 0.0;
        }
        finished.iter().sum::<u32>() as f32 / finished.len() as f32
    }
}

#[derive(Insertable)]
#[diesel(table_name=brackets)]
pub struct NewBracket {
    season_id: i32,
    name: String,
    state: String,
    bracket_type: String,
}

impl NewBracket {
    pub fn new<S: Into<String>>(season: &Season, name: S, type_: BracketType) -> Self {
        // ... okay this would be FINE to .unwrap(), but rules are rules
        let bracket_type = serde_json::to_string(&type_).unwrap_or("Unknown".to_string());
        let state =
            serde_json::to_string(&BracketState::Unstarted).unwrap_or("Unknown".to_string());
        Self {
            season_id: season.id,
            name: name.into(),
            state,
            bracket_type,
        }
    }

    save_fn!(brackets::table, Bracket);
}

#[cfg(test)]
mod tests {
    use crate::models::brackets::BracketState;
    use rocket::serde::json::serde_json;
    #[derive(Eq, PartialEq, Debug)]
    struct P {
        id: usize,
    }

    #[test]
    fn test_serialize() {
        assert_eq!(
            r#""Unstarted""#.to_string(),
            serde_json::to_string(&BracketState::Unstarted).unwrap()
        );
    }

    #[test]
    fn test_deserialize() {
        assert_eq!(
            BracketState::Unstarted,
            serde_json::from_str(r#""Unstarted""#).unwrap()
        );
    }
}
