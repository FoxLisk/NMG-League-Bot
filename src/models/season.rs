use chrono::{Duration, Utc};
use crate::models::brackets::Bracket;
use diesel::prelude::*;
use diesel::{RunQueryDsl, SqliteConnection};
use serde::Serialize;

use crate::schema::seasons;
use crate::utils::epoch_timestamp;
use crate::{save_fn, update_fn, NMGLeagueBotError, schema};
use enum_iterator::Sequence;
use crate::models::bracket_race_infos::BracketRaceInfo;
use crate::models::bracket_races::{BracketRace, BracketRaceState};

#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq, Debug, Sequence)]
pub enum SeasonState {
    Created,
    QualifiersOpen,
    QualifiersClosed,
    Started,
    Finished,
}

#[derive(Queryable, Debug, Serialize, Identifiable, AsChangeset)]
pub struct Season {
    pub id: i32,
    /// this is called 'started' but it should be called 'created'
    started: i64,
    finished: Option<i64>,
    /// this is called "format" but it's really just, like, the name of the season
    pub format: String,
    state: String,
    /// rt.gg calls games "categories"; this is e.g. alttp (or alttpr)
    pub rtgg_category_name: String,
    /// this is something like "Any% NMG". custom goals have their custom name in the same field,
    /// along with a "custom: true" field that I think we can maybe just ignore
    pub rtgg_goal_name: String,
}

impl Season {
    /// gets Season with this id (returns error if no season exists)
    pub fn get_by_id(id_: i32, conn: &mut SqliteConnection) -> Result<Self, diesel::result::Error> {
        use crate::schema::seasons::dsl::*;
        use diesel::prelude::*;
        seasons.filter(id.eq(id_)).first(conn)
    }

    pub fn get_active_season(
        conn: &mut SqliteConnection,
    ) -> Result<Option<Self>, diesel::result::Error> {
        use crate::schema::seasons::dsl::*;
        use diesel::prelude::*;
        seasons.filter(finished.is_null()).first(conn).optional()
    }

    pub fn are_qualifiers_open(&self) -> Result<bool, serde_json::Error> {
        Ok(SeasonState::QualifiersOpen == self.get_state()?)
    }

    fn get_state(&self) -> Result<SeasonState, serde_json::Error> {
        serde_json::from_str(&self.state)
    }

    /// this is a heavy duty function, not a normal setter. it will make sure state
    /// transitions are legal, check associated bracket states, etc
    pub fn set_state(
        &mut self,
        state: SeasonState,
        cxn: &mut SqliteConnection,
    ) -> Result<(), NMGLeagueBotError> {
        let current_state = self.get_state()?;
        if current_state == state {
            return Ok(());
        }
        macro_rules! expect_state {
            ($state:ident) => {
                if current_state != SeasonState::$state {
                    return Err(NMGLeagueBotError::StateError(format!(
                        "Expected state {}",
                        serde_json::to_string(&SeasonState::$state)?
                    )));
                }
            };
        }

        match state {
            SeasonState::Created => {
                return Err(NMGLeagueBotError::StateError(
                    "Seasons can't return to created".to_string(),
                ));
            }
            SeasonState::QualifiersOpen => {
                expect_state!(Created);
            }
            SeasonState::QualifiersClosed => {
                expect_state!(QualifiersOpen);
            }
            SeasonState::Started => {
                expect_state!(QualifiersClosed);
            }
            SeasonState::Finished => {
                expect_state!(Started);
                self.finish(cxn)?;
            }
        }
        self.state = serde_json::to_string(&state)?;
        Ok(())
    }

    pub fn brackets(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<Vec<Bracket>, diesel::result::Error> {
        use crate::schema::brackets as sbrack;
        sbrack::table
            .filter(sbrack::season_id.eq(self.id))
            .order_by(sbrack::id)
            .load(conn)
    }

    /// this checks all of its brackets for validity
    /// returns
    fn finish(&mut self, cxn: &mut SqliteConnection) -> Result<(), NMGLeagueBotError> {
        for b in self.brackets(cxn)? {
            if !b.is_finished()? {
                return Err(NMGLeagueBotError::StateError(format!(
                    "Cannot finish: bracket {} isn't finished yet.",
                    b.name
                )));
            }
        }
        self.finished = Some(epoch_timestamp() as i64);
        Ok(())
    }


    /// races that are not in finished state and that are scheduled to have started recently
    pub fn get_races_that_should_be_finishing_soon(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<Vec<(BracketRaceInfo, BracketRace)>, diesel::result::Error> {
        use schema::bracket_race_infos;
        use schema::bracket_races;

        let now = Utc::now();
        let start_time = now - Duration::minutes(82);
        // TODO: pretend to care about this unwrap later maybe
        let finished_state = serde_json::to_string(&BracketRaceState::Finished).unwrap();

        bracket_race_infos::table
            .inner_join(bracket_races::table)
            .filter(bracket_race_infos::scheduled_for.lt(start_time.timestamp()))
            .filter(bracket_races::state.ne(finished_state))
            .filter(bracket_races::id.eq(self.id))
            .load(conn)
    }


    update_fn! {}
}

#[derive(Insertable)]
#[diesel(table_name=seasons)]
pub struct NewSeason {
    pub format: String,
    pub started: i64,
    pub state: String,
    pub rtgg_category_name: String,
    pub rtgg_goal_name: String,
}

impl NewSeason {
    pub fn new<S: Into<String>>(format: S, rtgg_category_name: S, rtgg_goal_name: S) -> Self {
        Self {
            format: format.into(),
            started: epoch_timestamp() as i64,
            // TODO: unwrap
            state: serde_json::to_string(&SeasonState::Created).unwrap(),
            rtgg_category_name: rtgg_category_name.into(),
            rtgg_goal_name: rtgg_goal_name.into(),
        }
    }
    save_fn!(seasons::table, Season);
}
