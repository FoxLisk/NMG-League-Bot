use crate::models::brackets::Bracket;
use diesel::prelude::*;
use diesel::{RunQueryDsl, SqliteConnection};
use serde::Serialize;

use crate::utils::epoch_timestamp;
use crate::{NMGLeagueBotError, save_fn, update_fn};
use crate::schema::qualifier_submissions;
use enum_iterator::Sequence;
use crate::models::player::Player;
use crate::models::season::Season;

#[derive(Queryable, Debug, Serialize, Identifiable, AsChangeset)]
pub struct QualifierSubmission {
    pub id: i32,
    player_id: i32,
    season_id: i32,
    reported_time: i32,
    vod_link : String,
}

impl QualifierSubmission {

    update_fn!{}
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
