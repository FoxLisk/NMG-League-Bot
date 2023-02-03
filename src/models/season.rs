use crate::models::brackets::Bracket;
use diesel::prelude::*;
use diesel::{RunQueryDsl, SqliteConnection};
use serde::Serialize;

use crate::utils::epoch_timestamp;
use crate::{NMGLeagueBotError, save_fn, update_fn};
use crate::schema::seasons;

#[derive(Queryable, Debug, Serialize, Identifiable, AsChangeset)]
pub struct Season {
    pub id: i32,
    started: i64,
    finished: Option<i64>,
    pub format: String,
}

impl Season {
    /// gets Season with this id (returns error if no season exists)
    pub fn get_by_id(id_: i32, conn: &mut SqliteConnection) -> Result<Self, diesel::result::Error> {
        use crate::schema::seasons::dsl::*;
        use diesel::prelude::*;
        seasons
            .filter(id.eq(id_))
            .first(conn)
    }

    pub fn get_active_season(
        conn: &mut SqliteConnection,
    ) -> Result<Option<Self>, diesel::result::Error> {
        use crate::schema::seasons::dsl::*;
        use diesel::prelude::*;
        seasons.filter(finished.is_null()).first(conn).optional()
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
    pub fn finish(&mut self, cxn: &mut SqliteConnection) -> Result<bool, NMGLeagueBotError> {
        for b in self.brackets(cxn)? {
            if ! b.is_finished()? {
                return Ok(false);
            }
        }

        self.finished = Some(epoch_timestamp() as i64);
        Ok(true)
    }

    update_fn!{}
}


#[derive(Insertable)]
#[diesel(table_name=seasons)]
pub struct NewSeason {
    pub format: String,
    pub started: i64,
}

impl NewSeason {
    pub fn new<S: Into<String>>(format: S) -> Self {
        Self {
            format: format.into(),
            started: epoch_timestamp() as i64,
        }
    }
    save_fn!(seasons::table, Season);
}
