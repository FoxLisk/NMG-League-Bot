use diesel::prelude::Insertable;
use crate::models::epoch_timestamp;
use crate::models::player::Player;
use crate::models::season::Season;
use crate::schema::signups;


#[derive(Queryable)]
pub struct Signup {
    pub id: i32,
    player_id: i32,
    season_id: i32,
}



#[derive(Insertable)]
#[diesel(table_name=signups)]
pub struct NewSignup {
    player_id: i32,
    season_id: i32,
}

impl NewSignup {
    pub fn new(player: &Player, season: &Season) -> Self {
        Self {
            player_id: player.id,
            season_id: season.id,
        }
    }
}