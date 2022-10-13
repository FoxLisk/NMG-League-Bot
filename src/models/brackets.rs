use diesel::prelude::*;
use crate::models::season::Season;
use crate::save_fn;
use crate::schema::brackets;
use diesel::{SqliteConnection, RunQueryDsl};
use diesel::result::Error;
use diesel::sqlite::Sqlite;
use rand::thread_rng;
use crate::models::bracket_races::NewBracketRace;
use crate::models::bracket_rounds::NewBracketRound;
use crate::models::player::Player;
use crate::models::player_bracket_entries::PlayerBracketEntry;


#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq)]
enum BracketState {
    Unstarted,
    Started,
    Finished
}

#[derive(Queryable, Identifiable, Debug, AsChangeset)]
#[allow(unused)]
pub struct Bracket {
    pub id: i32,
    name: String,
    season_id: i32,
    state: String,
}

#[derive(Debug)]
pub enum PairingsError {
    InvalidState,
    OddPlayerCount,
    DBError(diesel::result::Error),
    Other(String)
}

impl From<diesel::result::Error> for PairingsError {
    fn from(e: Error) -> Self {
        Self::DBError(e)
    }
}

impl From<String> for PairingsError {
    fn from(e: String) -> Self {
        Self::Other(e)
    }
}

impl Bracket {

    fn state(&self) -> Result<BracketState, serde_json::Error> {
        serde_json::from_str(&self.state)
    }

    fn set_state(&mut self, state: BracketState) -> Result<(), serde_json::Error> {
        self.state = serde_json::to_string(&state)?;
        Ok(())
    }

    pub fn update(&self, conn: &mut SqliteConnection) -> QueryResult<usize> {
        diesel::update(self)
            .set(self)
            .execute(conn)
    }

    pub fn players(&self, conn: &mut SqliteConnection) -> Result<Vec<Player>, diesel::result::Error> {
        use crate::schema::player_bracket_entry as pbes;
        use crate::schema::players;
        pbes::table.filter(pbes::bracket_id.eq(self.id))
            .inner_join(players::table)
            .select(players::all_columns)
            .load(conn)
    }

    fn generate_initial_pairings(&mut self, conn: &mut SqliteConnection) -> Result<(), PairingsError> {
        use rand::seq::SliceRandom;
        use crate::schema::bracket_races;
        use crate::schema::bracket_rounds;

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
        if ! players.is_empty() {
            return Err(PairingsError::OddPlayerCount);
        }
        diesel::insert_into(bracket_races::table)
            .values(&nbrs)
            .execute(conn)?;
        self.set_state(BracketState::Started).map_err(|e| e.to_string())?;
        self.update(conn)?;
        Ok(())
    }

    fn generate_next_round_pairings(&self, conn: &mut SqliteConnection) -> Result<(), PairingsError> {
        todo!();
    }


    /// creates and saves a bunch of BracketRaces representing the next round's matchups
    pub fn generate_pairings(&mut self, conn: &mut SqliteConnection) -> Result<(), PairingsError> {
        let state = self.state().map_err(|_| PairingsError::InvalidState)?;
        match state {
            BracketState::Unstarted => {self.generate_initial_pairings(conn)}
            BracketState::Started => {self.generate_next_round_pairings(conn)}
            BracketState::Finished => {Err(PairingsError::InvalidState)}
        }
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
            state: serde_json::to_string(&BracketState::Unstarted).unwrap_or("Unknown".to_string())
        }
    }

    save_fn!(brackets::table, Bracket);
}

#[cfg(test)]
mod tests {
    use rocket::serde::json::serde_json;
    use crate::models::brackets::BracketState;

    #[test]
    fn test_serialize() {
        assert_eq!(r#""Unstarted""#.to_string(), serde_json::to_string(&BracketState::Unstarted).unwrap());
    }

    #[test]
    fn test_deserialize() {
        assert_eq!(BracketState::Unstarted, serde_json::from_str(r#""Unstarted""#));
    }
}