use crate::models::brackets::Bracket;
use diesel::prelude::*;
use diesel::{RunQueryDsl, SqliteConnection};
use serde::Serialize;

use crate::schema::seasons;
use crate::utils::epoch_timestamp;
use crate::{save_fn, update_fn, NMGLeagueBotError};
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
    state: String,
}

impl Season {
    pub fn get_by_id(id_: i32, conn: &mut SqliteConnection) -> Result<Self, String> {
        use crate::schema::seasons::dsl::*;
        use diesel::prelude::*;
        seasons
            .filter(id.eq(id_))
            .first(conn)
            .map_err(|e| e.to_string())
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

    update_fn! {}
}

#[derive(Insertable)]
#[diesel(table_name=seasons)]
pub struct NewSeason {
    pub format: String,
    pub started: i64,
    pub state: String,
}

impl NewSeason {
    pub fn new<S: Into<String>>(format: S) -> Self {
        Self {
            format: format.into(),
            started: epoch_timestamp() as i64,
            // TODO: unwrap
            state: serde_json::to_string(&SeasonState::Created).unwrap(),
        }
    }
    save_fn!(seasons::table, Season);
}
