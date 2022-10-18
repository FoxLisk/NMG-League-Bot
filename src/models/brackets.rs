use crate::models::bracket_races::{
    insert_bulk, BracketRace, BracketRaceStateError, MatchResultError, NewBracketRace,
};
use crate::models::bracket_rounds::{BracketRound, NewBracketRound};
use crate::models::player::Player;
use crate::models::season::Season;
use crate::schema::brackets;
use crate::{save_fn, update_fn};
use diesel::prelude::*;
use diesel::result::Error;
use diesel::{RunQueryDsl, SqliteConnection};
use rand::thread_rng;
use serde::Serialize;
use std::collections::HashMap;
use swiss_pairings::{PairingError, TourneyConfig};

#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq)]
enum BracketState {
    Unstarted,
    Started,
    Finished,
}

#[derive(Queryable, Identifiable, Debug, AsChangeset, Serialize)]
#[allow(unused)]
pub struct Bracket {
    pub id: i32,
    pub name: String,
    season_id: i32,
    state: String,
}

#[derive(Debug)]
pub enum BracketError {
    InvalidState,
    OddPlayerCount,
    DBError(diesel::result::Error),
    Other(String),
    BracketRaceStateError(BracketRaceStateError),
    SerializationError(serde_json::Error),
    MatchResultError(MatchResultError),
    PairingError(PairingError),
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
impl From<serde_json::Error> for BracketError {
    fn from(e: serde_json::Error) -> Self {
        Self::SerializationError(e)
    }
}
impl From<diesel::result::Error> for BracketError {
    fn from(e: Error) -> Self {
        Self::DBError(e)
    }
}

impl From<String> for BracketError {
    fn from(e: String) -> Self {
        Self::Other(e)
    }
}

impl From<BracketRaceStateError> for BracketError {
    fn from(e: BracketRaceStateError) -> Self {
        BracketError::BracketRaceStateError(e)
    }
}

fn generate_next_round_pairings(
    bracket: &Bracket,
    conn: &mut SqliteConnection,
) -> Result<(), BracketError> {
    if bracket.state()? != BracketState::Started {
        return Err(BracketError::InvalidState);
    }
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
    println!("{:?}", pairing_rounds);
    let cfg = TourneyConfig {
        points_per_win: 2,
        points_per_loss: 0,
        points_per_draw: 1,
        error_on_repeated_opponent: true,
    };
    let (pairings, _standings) = swiss_pairings::swiss_pairings(
        &pairing_rounds,
        &cfg,
        swiss_pairings::random_by_scoregroup,
    )?;
    println!("{:?}", pairings);

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

impl Bracket {
    fn state(&self) -> Result<BracketState, serde_json::Error> {
        serde_json::from_str(&self.state)
    }

    fn set_state(&mut self, state: BracketState) -> Result<(), serde_json::Error> {
        self.state = serde_json::to_string(&state)?;
        Ok(())
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

    fn generate_initial_pairings(
        &mut self,
        conn: &mut SqliteConnection,
    ) -> Result<(), BracketError> {
        use rand::seq::SliceRandom;

        if self.state()? != BracketState::Unstarted {
            return Err(BracketError::InvalidState);
        }
        if let Some(_) = self.current_round(conn)? {
            return Err(BracketError::InvalidState);
        }

        let new_round = NewBracketRound::new(self, 1);
        let round = new_round.save(conn)?;

        let mut players = self.players(conn)?;
        players.as_mut_slice().shuffle(&mut thread_rng());
        let mut nbrs = vec![];
        while players.len() > 1 {
            let p1 = players.pop().unwrap();
            let p2 = players.pop().unwrap();
            let nbr = NewBracketRace::new(self, &round, &p1, &p2);
            nbrs.push(nbr);
        }
        if !players.is_empty() {
            return Err(BracketError::OddPlayerCount);
        }
        insert_bulk(&nbrs, conn)?;

        self.set_state(BracketState::Started)
            .map_err(|e| e.to_string())?;
        self.update(conn)?;
        Ok(())
    }

    /// creates and saves a bunch of BracketRaces representing the next round's matchups
    pub fn generate_pairings(&mut self, conn: &mut SqliteConnection) -> Result<(), BracketError> {
        let state = self.state().map_err(|_| BracketError::InvalidState)?;
        match state {
            BracketState::Unstarted => self.generate_initial_pairings(conn),
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
}

#[derive(Insertable)]
#[diesel(table_name=brackets)]
pub struct NewBracket {
    season_id: i32,
    name: String,
    state: String,
}

impl NewBracket {
    pub fn new<S: Into<String>>(season: &Season, name: S) -> Self {
        // ... okay this would be FINE to .unwrap(), but rules are rules
        Self {
            season_id: season.id,
            name: name.into(),
            state: serde_json::to_string(&BracketState::Unstarted).unwrap_or("Unknown".to_string()),
        }
    }

    save_fn!(brackets::table, Bracket);
}

#[cfg(test)]
mod tests {
    use crate::models::brackets::BracketState;
    use rocket::serde::json::serde_json;

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
            serde_json::from_str(r#""Unstarted""#)
        );
    }
}
