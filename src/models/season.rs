use crate::models::brackets::Bracket;
use chrono::{Duration, Utc};
use diesel::prelude::*;
use diesel::{RunQueryDsl, SqliteConnection};
use serde::Serialize;

use crate::models::bracket_race_infos::BracketRaceInfo;
use crate::models::bracket_races::{BracketRace, BracketRaceState};
use crate::schema::seasons;
use crate::utils::epoch_timestamp;
use crate::{save_fn, schema, update_fn, NMGLeagueBotError};
use enum_iterator::Sequence;

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
    pub format: String,
    pub ordinal: i32,
    state: String,
    /// rt.gg calls games "categories"; this is e.g. alttp (or alttpr)
    pub rtgg_category_name: String,
    /// this is something like "Any% NMG". custom goals have their custom name in the same field,
    /// along with a "custom: true" field that I think we can maybe just ignore
    pub rtgg_goal_name: String,
}

impl Season {
    /// gets Season with this id (returns error if no season exists)
    ///
    /// You should VERY STRONGLY prefer [`get_by_ordinal`] in most use cases
    pub fn get_by_id(id_: i32, conn: &mut SqliteConnection) -> Result<Self, diesel::result::Error> {
        use crate::schema::seasons::dsl::*;
        use diesel::prelude::*;
        seasons.filter(id.eq(id_)).first(conn)
    }

    /// gets Season with this ordinal (returns error if no season exists)
    pub fn get_by_ordinal(
        ordinal_: i32,
        conn: &mut SqliteConnection,
    ) -> Result<Self, diesel::result::Error> {
        use crate::schema::seasons::dsl::*;
        use diesel::prelude::*;
        seasons.filter(ordinal.eq(ordinal_)).first(conn)
    }

    pub fn get_active_season(
        conn: &mut SqliteConnection,
    ) -> Result<Option<Self>, diesel::result::Error> {
        use crate::schema::seasons::dsl::*;
        use diesel::prelude::*;
        seasons.filter(finished.is_null()).first(conn).optional()
    }

    pub fn get_from_bracket_race_info(
        bri: &BracketRaceInfo,
        conn: &mut SqliteConnection,
    ) -> Result<Self, diesel::result::Error> {
        use crate::schema::bracket_race_infos;
        use crate::schema::bracket_races;
        use crate::schema::brackets;
        use diesel::prelude::*;

        let szn = seasons::table
            .inner_join(
                brackets::table
                    .inner_join(bracket_races::table.inner_join(bracket_race_infos::table)),
            )
            .filter(bracket_race_infos::columns::bracket_race_id.eq(bri.id))
            .select(seasons::all_columns)
            .first(conn)?;

        Ok(szn)
    }

    pub fn are_qualifiers_open(&self) -> Result<bool, serde_json::Error> {
        Ok(SeasonState::QualifiersOpen == self.get_state()?)
    }

    pub fn get_state(&self) -> Result<SeasonState, serde_json::Error> {
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
        use schema::brackets;

        let now = Utc::now();
        // TODO: this should be configurable or we should stop caring about it, maybe
        let start_time = now - Duration::minutes(70);
        // TODO: pretend to care about this unwrap later maybe
        let finished_state = serde_json::to_string(&BracketRaceState::Finished).unwrap();

        bracket_race_infos::table
            .inner_join(bracket_races::table.inner_join(brackets::table))
            .select((bracket_race_infos::all_columns, bracket_races::all_columns))
            .filter(bracket_race_infos::scheduled_for.lt(start_time.timestamp()))
            .filter(bracket_races::state.ne(finished_state))
            .filter(brackets::season_id.eq(self.id))
            .load(conn)
    }

    pub fn get_unfinished_races(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<Vec<(BracketRaceInfo, BracketRace)>, diesel::result::Error> {
        use schema::bracket_race_infos;
        use schema::bracket_races;
        use schema::brackets;
        let finished_state = serde_json::to_string(&BracketRaceState::Finished).unwrap();

        bracket_race_infos::table
            .inner_join(bracket_races::table.inner_join(brackets::table))
            .select((bracket_race_infos::all_columns, bracket_races::all_columns))
            .filter(bracket_races::state.ne(finished_state))
            .filter(brackets::season_id.eq(self.id))
            .load(conn)
    }

    /// for finding races that are about to start or are in progress
    pub fn get_unfinished_races_starting_before(
        &self,
        when: i64,
        conn: &mut SqliteConnection,
    ) -> Result<Vec<(BracketRaceInfo, BracketRace)>, diesel::result::Error> {
        use schema::bracket_race_infos;
        use schema::bracket_races;
        use schema::brackets;

        // TODO: pretend to care about this unwrap later maybe
        let finished_state = serde_json::to_string(&BracketRaceState::Finished).unwrap();

        bracket_race_infos::table
            .inner_join(bracket_races::table.inner_join(brackets::table))
            .select((bracket_race_infos::all_columns, bracket_races::all_columns))
            .filter(bracket_race_infos::scheduled_for.lt(when))
            .filter(bracket_races::state.ne(finished_state))
            .filter(brackets::season_id.eq(self.id))
            .load(conn)
    }

    pub fn safe_to_delete_qualifiers(&self) -> Result<bool, NMGLeagueBotError> {
        match self.get_state()? {
            SeasonState::QualifiersOpen | SeasonState::QualifiersClosed => Ok(true),
            SeasonState::Created | SeasonState::Started | SeasonState::Finished => Ok(false),
        }
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
    pub ordinal: i32,
}

impl NewSeason {
    pub fn new<S: Into<String>>(
        format: S,
        rtgg_category_name: S,
        rtgg_goal_name: S,
        conn: &mut SqliteConnection,
    ) -> Result<Self, diesel::result::Error> {
        use diesel::dsl::max;
        let ordinal: i32 = (seasons::table
            .select(max(schema::seasons::dsl::ordinal))
            .first::<Option<i32>>(conn)?)
        .unwrap_or(0)
            + 1;
        Ok(Self {
            format: format.into(),
            started: epoch_timestamp() as i64,
            // TODO: unwrap
            state: serde_json::to_string(&SeasonState::Created).unwrap(),
            rtgg_category_name: rtgg_category_name.into(),
            rtgg_goal_name: rtgg_goal_name.into(),
            ordinal,
        })
    }
    save_fn!(seasons::table, Season);
}

#[cfg(test)]
impl Season {
    pub fn new(id: i32, goal: &str) -> Self {
        Season {
            id,
            ordinal: id,
            started: 0,
            finished: None,
            format: "".to_string(),
            state: "".to_string(),
            rtgg_category_name: "".to_string(),
            rtgg_goal_name: goal.to_string(),
        }
    }
}
