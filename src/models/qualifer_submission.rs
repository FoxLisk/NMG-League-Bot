use diesel::prelude::*;
use diesel::RunQueryDsl;
use serde::Serialize;

use crate::models::player::Player;
use crate::models::season::Season;
use crate::schema::qualifier_submissions;
use crate::{delete_fn, save_fn, update_fn, NMGLeagueBotError};

#[derive(Queryable, Debug, Serialize, Identifiable, AsChangeset)]
pub struct QualifierSubmission {
    pub id: i32,
    player_id: i32,
    season_id: i32,
    reported_time: i32,
    vod_link: String,
}

impl QualifierSubmission {
    pub fn get_by_id(id: i32, conn: &mut SqliteConnection) -> Result<Self, diesel::result::Error> {
        qualifier_submissions::table.find(id).first(conn)
    }
}

impl QualifierSubmission {
    /// checks if it's safe to delete this submission (i.e. the season it's part of is in a state
    /// where deleting qualifiers is reasonable)
    /// you can use [Season::safe_to_delete_qualifiers] if you have a Season in hand already
    pub fn safe_to_delete(&self, conn: &mut SqliteConnection) -> Result<bool, NMGLeagueBotError> {
        Season::get_by_id(self.season_id, conn)?.safe_to_delete_qualifiers()
    }
    update_fn! {}
    delete_fn!(qualifier_submissions::table);
}

#[derive(Insertable)]
#[diesel(table_name=qualifier_submissions)]
pub struct NewQualifierSubmission {
    player_id: i32,
    season_id: i32,
    reported_time: i32,
    vod_link: String,
}

impl NewQualifierSubmission {
    pub fn new(player: &Player, season: &Season, reported_time: u32, vod_link: String) -> Self {
        Self {
            player_id: player.id,
            season_id: season.id,
            reported_time: reported_time as i32,
            vod_link,
        }
    }
    save_fn!(qualifier_submissions::table, QualifierSubmission);
}
