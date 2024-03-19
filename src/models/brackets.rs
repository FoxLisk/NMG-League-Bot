use crate::models::bracket_races::{
    insert_bulk, BracketRace, BracketRaceStateError, MatchResultError, NewBracketRace, Outcome,
};
use crate::models::bracket_rounds::{BracketRound, NewBracketRound};
use crate::models::player::Player;
use crate::models::season::Season;
use crate::schema::brackets;
use crate::{save_fn, update_fn, NMGLeagueBotError};
use diesel::prelude::*;
use diesel::result::Error;
use diesel::{RunQueryDsl, SqliteConnection};
use enum_iterator::Sequence;
use itertools::Itertools;
use log::{debug, warn};
use rand::thread_rng;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::time::Duration;
use swiss_pairings::{PairingError, TourneyConfig};
use thiserror::Error;

use rand::seq::SliceRandom;

use super::bracket_races::PlayerResult;

#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq, Debug)]
enum BracketState {
    Unstarted,
    Started,
    Finished,
}

#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq, Debug, Sequence)]
pub enum BracketType {
    Swiss,
    RoundRobin,
}

#[derive(Queryable, Identifiable, Debug, AsChangeset, Serialize)]
#[allow(unused)]
pub struct Bracket {
    pub id: i32,
    pub name: String,
    pub season_id: i32,
    state: String,
    bracket_type: String,
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

fn generate_next_round_pairings_round_robin(
    bracket: &Bracket,
    conn: &mut SqliteConnection,
) -> Result<(), BracketError> {
    let rounds = bracket.rounds(conn)?;
    let mut highest_round_num = 0;
    for round in rounds {
        assert!(round.round_num > highest_round_num);
        highest_round_num = round.round_num;
    }
    let new_round = NewBracketRound::new(bracket, highest_round_num + 1);
    let round = new_round.save(conn)?;
    generate_pairings_round_robin(bracket, &round, conn)
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
        BracketType::RoundRobin => generate_next_round_pairings_round_robin(bracket, conn),
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

/// constructs the polygon of size `n` if `n` is odd `n-1` if `n` is even
/// rotates it counterclockwise `rotation` times
/// returns the pairings across the polgyon and the leftover vertex index
/// returns None if your input sucks shit
fn polygon_indices(n: usize, rotation: usize) -> Option<(Vec<(usize, usize)>, usize)> {
    if n < 2 {
        return None;
    }
    let vertices = if n % 2 == 1 { n } else { n - 1 };
    /*
    we imagine a polygon with N sides (N-1 if N even). We number vertices
    clockwise from the bottom left, with N marked in the middle
            2
           / \
        1 | 5 |3
        0 |___|4

    in each round, we rotate the vertex labels clockwise once. e.g.
            1
           / \
        0 | 5 |2
        4 |___|3

    players play the person across from them on the polygon, with the left out player at the top
    vertex playing the middle player
     */
    let rotate = |idx: usize| {
        // idx + rotation corresponds to a counterclockwise rotation of points;
        // idx - rotation would be anticlockwise (same pairings in the end but they come up
        // in a different order), however it runs into issues because usize doesn't handle
        // negative numbers (obviously!)
        (idx + rotation) % vertices
    };
    let mut indices = (0..vertices).map(|i| rotate(i)).collect::<VecDeque<_>>();
    let mut index_pairs = vec![];
    while indices.len() > 1 {
        let i1 = indices.pop_front()?;
        let i2 = indices.pop_back()?;
        index_pairs.push((i1, i2));
    }
    let middle = indices.pop_front()?;
    Some((index_pairs, middle))
}

/// players MUST be sorted the same way on each call to this method!
/// Returns None if players is empty or some shit
fn polygon_method<T>(players: &Vec<T>, round: usize) -> Option<Vec<(&T, &T)>> {
    // https://web.archive.org/web/20230401000000*/https://nrich.maths.org/1443
    let n = players.len();
    if round < 1 {
        return None;
    }
    let (pairs, middle) = polygon_indices(n, round - 1)?;
    let mut player_pairs = vec![];
    for (i1, i2) in pairs {
        player_pairs.push((players.get(i1)?, players.get(i2)?));
    }
    if n % 2 == 0 {
        player_pairs.push((players.get(middle)?, players.get(n - 1)?));
    }
    Some(player_pairs)
}

fn generate_pairings_round_robin(
    bracket: &Bracket,
    round: &BracketRound,
    conn: &mut SqliteConnection,
) -> Result<(), BracketError> {
    let mut players = bracket.players(conn)?;
    players.sort_unstable_by_key(|p| p.id);
    let pairings = polygon_method(&players, round.round_num as usize).ok_or(
        BracketError::RoundRobinError("Unable to generate pairings".to_string()),
    )?;
    let mut nbrs = vec![];
    for (p1, p2) in pairings {
        nbrs.push(NewBracketRace::new(bracket, &round, p1, p2));
    }
    insert_bulk(&nbrs, conn)?;

    Ok(())
}

fn generate_initial_pairings_round_robin(
    bracket: &mut Bracket,
    conn: &mut SqliteConnection,
) -> Result<(), BracketError> {
    let new_round = NewBracketRound::new(bracket, 1);
    let round = new_round.save(conn)?;
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
    fn state(&self) -> Result<BracketState, serde_json::Error> {
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
        for round in rounds {
            let round_races = round.races(conn)?;
            if !all_races_complete(&round_races) {
                // this either means we've reached the current round, or it means that
                // we have a data issue, and in either case i'm giving up
                break;
            }
            races.extend(round_races);
        }

        let mut info: HashMap<i32, PlayerInfoBuilder> = Default::default();
        for race in races {
            let p1r = race.player_1_result().ok_or(BracketError::InvalidState)?;
            let p2r = race.player_2_result().ok_or(BracketError::InvalidState)?;
            let o = race.outcome()?.ok_or(BracketError::InvalidState)?;
            let (p1_adjust, p2_adjust) = match o {
                Outcome::Tie => (1, 1),
                Outcome::P1Win => (2, 0),
                Outcome::P2Win => (0, 2),
            };
            let p1_i_b = info
                .entry(race.player_1_id)
                .or_insert(PlayerInfoBuilder::new(race.player_1_id));
            p1_i_b.results.push(p1r);
            p1_i_b.points += p1_adjust;
            p1_i_b.opponents.push(race.player_2_id);

            let p2_i_b = info
                .entry(race.player_2_id)
                .or_insert(PlayerInfoBuilder::new(race.player_2_id));
            p2_i_b.results.push(p2r);
            p2_i_b.points += p2_adjust;
            p2_i_b.opponents.push(race.player_1_id);
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

fn all_races_complete(races: &[BracketRace]) -> bool {
    for race in races {
        match race.outcome() {
            Ok(Some(_)) => {}
            _ => {
                return false;
            }
        }
    }
    true
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
    use crate::models::brackets::{polygon_method, BracketState};
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

    #[test]
    fn test_polygon_method_four() {
        let p1 = P { id: 1 };
        let p2 = P { id: 2 };
        let p3 = P { id: 3 };
        let p4 = P { id: 4 };
        let players = vec![p1, p2, p3, p4];
        let r1 = polygon_method(&players, 1).unwrap();
        let r2 = polygon_method(&players, 2).unwrap();
        let r3 = polygon_method(&players, 3).unwrap();
        let r1_pairs = r1
            .iter()
            .map(|(pp1, pp2)| (pp1.id, pp2.id))
            .collect::<Vec<_>>();
        let r2_pairs = r2
            .iter()
            .map(|(pp1, pp2)| (pp1.id, pp2.id))
            .collect::<Vec<_>>();
        let r3_pairs = r3
            .iter()
            .map(|(pp1, pp2)| (pp1.id, pp2.id))
            .collect::<Vec<_>>();
        assert_eq!(vec![(1, 3), (2, 4)], r1_pairs);
        assert_eq!(vec![(2, 1), (3, 4)], r2_pairs);
        assert_eq!(vec![(3, 2), (1, 4)], r3_pairs);
    }

    #[test]
    fn test_polygon_method_six() {
        let players = (1..7).map(|id| P { id }).collect::<Vec<_>>();
        let rounds = (1..=5)
            .map(|r| {
                polygon_method(&players, r)
                    .unwrap()
                    .iter()
                    .map(|(p1, p2)| (p1.id, p2.id))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            vec![
                vec![(1, 5), (2, 4), (3, 6)],
                vec![(2, 1), (3, 5), (4, 6)],
                vec![(3, 2), (4, 1), (5, 6)],
                vec![(4, 3), (5, 2), (1, 6)],
                vec![(5, 4), (1, 3), (2, 6)],
            ],
            rounds
        );
    }

    #[test]
    fn test_polygon_method_three() {
        let players = (1..=3).map(|id| P { id }).collect::<Vec<_>>();
        let rounds = (1..=3)
            .map(|r| {
                polygon_method(&players, r)
                    .unwrap()
                    .iter()
                    .map(|(p1, p2)| (p1.id, p2.id))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(vec![vec![(1, 3)], vec![(2, 1)], vec![(3, 2)],], rounds);
    }

    #[test]
    fn test_polygon_method_five() {
        let players = (1..=5).map(|id| P { id }).collect::<Vec<_>>();
        let rounds = (1..=5)
            .map(|r| {
                polygon_method(&players, r)
                    .unwrap()
                    .iter()
                    .map(|(p1, p2)| (p1.id, p2.id))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            vec![
                vec![(1, 5), (2, 4)],
                vec![(2, 1), (3, 5)],
                vec![(3, 2), (4, 1)],
                vec![(4, 3), (5, 2)],
                vec![(5, 4), (1, 3)],
            ],
            rounds
        );
    }
}
