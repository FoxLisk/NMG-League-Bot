use std::process::id;
use diesel::prelude::Insertable;
use diesel::SqliteConnection;
use crate::models::epoch_timestamp;
// use crate::schema::players::dsl::*;
use crate::schema::seasons;

#[derive(Queryable)]
pub struct Season {
    pub id: i32,
    started: i64,
    finished: Option<i64>,
    pub format: String,
}

impl Season {
    pub fn get_by_id(
        id_: i32,
        conn: &mut SqliteConnection,
    ) -> Result<Self, String> {
        use crate::schema::seasons::dsl::*;
        use diesel::prelude::*;
        seasons
            .filter(id.eq(id_))
            .first(conn)
            .map_err(|e| e.to_string())
    }

    pub fn get_active_season(conn: &mut SqliteConnection) -> Result<Option<Self>, diesel::result::Error> {
        use crate::schema::seasons::dsl::*;
        use diesel::prelude::*;
        seasons
            .filter(finished.is_null())
            .first(conn)
            .optional()
    }
}


#[derive(Insertable)]
#[diesel(table_name=seasons)]
pub struct NewSeason {
    pub format: String,
    pub started: i64,
}

impl NewSeason {
    pub fn new(format: String) -> Self {
        Self {
            format,
            started: epoch_timestamp() as i64
        }
    }
}

// use
//
// struct