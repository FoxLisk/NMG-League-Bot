use crate::models::bracket_races::BracketRace;
use crate::models::bracket_rounds::BracketRound;
use crate::models::brackets::Bracket;
use crate::models::player::Player;
use crate::models::season::Season;
use crate::schema::bracket_race_infos;
use crate::utils::format_hms;
use crate::{save_fn, update_fn};
use chrono::{DateTime, TimeZone, Utc};
use diesel::prelude::*;
use diesel::SqliteConnection;
use serde::Serialize;
use serde_json::Error;


#[derive(Queryable, Identifiable, Debug, AsChangeset, Serialize)]
pub struct BracketRaceInfo {
    id: i32,
    bracket_race_id: i32,
    scheduled_for: Option<i64>,
}

impl BracketRaceInfo {
    pub fn get_or_create_for_bracket(
        bracket_race: &BracketRace,
        conn: &mut SqliteConnection,
    ) -> Result<Self, diesel::result::Error> {
        conn.transaction(|conn| {
            match bracket_race_infos::table
                .filter(bracket_race_infos::bracket_race_id.eq(bracket_race.id))
                .first(conn)
                .optional()?
            {
                Some(bi) => Ok(bi),
                None => NewBracketRaceInfo::new(bracket_race, None).save(conn),
            }
        })
    }

    pub fn scheduled(&self) -> Option<DateTime<Utc>> {
        self.scheduled_for.map(|t| Utc.timestamp(t, 0))
    }


    /// returns the prior scheduled time, if any (as timestamp)
    pub fn schedule<T: TimeZone>(
        &mut self,
        when: &DateTime<T>,
    ) -> Option<i64> {
        std::mem::replace(&mut self.scheduled_for, Some(when.timestamp()))
    }

    update_fn! {}
}

#[derive(Insertable)]
#[diesel(table_name=bracket_race_infos)]
pub struct NewBracketRaceInfo {
    bracket_race_id: i32,
    scheduled_for: Option<i64>,
}

impl NewBracketRaceInfo {
    pub fn new(bracket_race: &BracketRace, scheduled_for: Option<i64>) -> Self {
        Self {
            bracket_race_id: bracket_race.id,
            scheduled_for,
        }
    }

    save_fn!(bracket_race_infos::table, BracketRaceInfo);
}
