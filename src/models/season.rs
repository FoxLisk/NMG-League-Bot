use crate::models::brackets::Bracket;
use chrono::{Duration, Utc};
use diesel::prelude::*;
use diesel::{RunQueryDsl, SqliteConnection};
use serde::Serialize;

use crate::models::bracket_race_infos::BracketRaceInfo;
use crate::models::bracket_races::BracketRace;
use crate::schema::seasons;
use crate::utils::epoch_timestamp;
use crate::{save_fn, schema, update_fn, BracketRaceState};
use enum_iterator::Sequence;
use thiserror::Error;

#[derive(Copy, Clone, serde::Serialize, serde::Deserialize, Eq, PartialEq, Debug, Sequence)]
pub enum SeasonState {
    Created,
    QualifiersOpen,
    QualifiersClosed,
    Started,
    Finished,
}

#[derive(Debug, Error)]
pub enum SeasonStateTransitionError {
    #[error("{context}: {source}")]
    StateSerialization {
        context: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("expected season state {expected:?}, found {actual:?}")]
    InvalidTransition {
        expected: SeasonState,
        actual: SeasonState,
    },
    #[error("seasons cannot return to the Created state")]
    CannotReturnToCreated,
    #[error("database operation failed: {0}")]
    Database(#[from] diesel::result::Error),
    #[error("cannot finish the season because bracket `{0}` is not finished")]
    BracketNotFinished(String),
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
            .filter(bracket_race_infos::columns::id.eq(bri.id))
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
    ) -> Result<(), SeasonStateTransitionError> {
        let current_state =
            self.get_state()
                .map_err(|source| SeasonStateTransitionError::StateSerialization {
                    context: "could not decode the current season state".to_string(),
                    source,
                })?;
        if current_state == state {
            return Ok(());
        }
        macro_rules! expect_state {
            ($state:ident) => {
                if current_state != SeasonState::$state {
                    return Err(SeasonStateTransitionError::InvalidTransition {
                        expected: SeasonState::$state,
                        actual: current_state,
                    });
                }
            };
        }

        match state {
            SeasonState::Created => {
                return Err(SeasonStateTransitionError::CannotReturnToCreated);
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
        self.state = serde_json::to_string(&state).map_err(|source| {
            SeasonStateTransitionError::StateSerialization {
                context: "could not encode the new season state".to_string(),
                source,
            }
        })?;
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
    fn finish(&mut self, cxn: &mut SqliteConnection) -> Result<(), SeasonStateTransitionError> {
        for b in self.brackets(cxn)? {
            let is_finished = b.is_finished().map_err(|source| {
                SeasonStateTransitionError::StateSerialization {
                    context: format!("could not decode state for bracket `{}`", b.name),
                    source,
                }
            })?;
            if !is_finished {
                return Err(SeasonStateTransitionError::BracketNotFinished(b.name));
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

    pub fn safe_to_delete_qualifiers(&self) -> Result<bool, serde_json::Error> {
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
    /// this requires a database connection to get the next ordinal
    // TODO: probably technically a race condition in here lmao
    pub fn new<S: Into<String>>(
        format: S,
        rtgg_category_name: S,
        rtgg_goal_name: S,
        conn: &mut SqliteConnection,
    ) -> Result<Self, diesel::result::Error> {
        let ordinal: i32 = (seasons::table
            .select(diesel::dsl::max(schema::seasons::dsl::ordinal))
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
mod tests {
    use super::{NewSeason, Season};
    use crate::models::bracket_races::NewBracketRace;
    use crate::models::bracket_rounds::NewBracketRound;
    use crate::models::brackets::{BracketType, NewBracket};
    use crate::models::player::NewPlayer;
    use crate::test_utils::setup_db;

    #[test]
    fn gets_season_by_bracket_race_info_id() -> anyhow::Result<()> {
        let mut conn = setup_db()?;
        let season = NewSeason::new("Test", "alttp", "Any% NMG", &mut conn)?.save(&mut conn)?;
        let bracket = NewBracket::new(&season, "Test", BracketType::Swiss).save(&mut conn)?;
        let round = NewBracketRound::new(&bracket, 1).save(&mut conn)?;
        let player_1 = NewPlayer::new("Player 1", "1", None, None, None).save(&mut conn)?;
        let player_2 = NewPlayer::new("Player 2", "2", None, None, None).save(&mut conn)?;

        // Consume the first race ID so the target race and its first info row have different IDs.
        NewBracketRace::new(&bracket, &round, &player_1, &player_2).save(&mut conn)?;
        let target_race =
            NewBracketRace::new(&bracket, &round, &player_1, &player_2).save(&mut conn)?;
        let info = target_race.info(&mut conn)?;
        assert_ne!(info.id, info.bracket_race_id);

        let found = Season::get_from_bracket_race_info(&info, &mut conn)?;
        assert_eq!(found.id, season.id);
        Ok(())
    }
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

#[cfg(feature = "development")]
impl Season {
    pub fn ensure_started_season(conn: &mut SqliteConnection) -> anyhow::Result<Self> {
        if let Some(s) = Season::get_active_season(conn)? {
            return Ok(s);
        }

        let nsn = NewSeason::new("Test NMG", "alttp", "Any% NMG", conn)?;
        let mut sn = nsn.save(conn).unwrap();
        sn.set_state(SeasonState::QualifiersOpen, conn)?;
        sn.set_state(SeasonState::QualifiersClosed, conn)?;
        sn.set_state(SeasonState::Started, conn)?;
        sn.update(conn)?;
        Ok(sn)
    }
}
