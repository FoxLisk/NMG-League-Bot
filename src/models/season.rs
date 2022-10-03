use diesel::prelude::Insertable;
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
