//! api lol. the idea is just stuff that returns json i guess

use diesel::{QueryDsl, SqliteConnection};
use diesel::result::Error;
use rocket::{Build, get, Rocket};
use rocket::serde::json::Json;
use nmg_league_bot::models::qualifer_submission::QualifierSubmission;
use nmg_league_bot::models::season::Season;
use nmg_league_bot::NMGLeagueBotError;
use crate::web::ConnectionWrapper;

#[derive(diesel::Queryable, serde::Serialize)]
struct Qualifier {
    player_name: String,
    time: i32,
    vod: String,
}

#[derive(serde::Serialize, thiserror::Error, Debug)]
enum ApiError {
    #[error("Internal error communicating with database")]
    DatabaseError,
}

impl From<diesel::result::Error> for ApiError {
    fn from(value: Error) -> Self {
        Self::DatabaseError
    }
}

fn get_qualifiers(id: i32,  db: &mut SqliteConnection) -> Result<Vec<Qualifier>, ApiError> {
    use diesel::prelude::*;
    use crate::schema::{qualifier_submissions as qs, players};
    Ok(qs::table.inner_join(players::table)
        .filter(qs::season_id.eq(id))
        .select( (players::name, qs::reported_time, qs::vod_link))
        .order_by(qs::reported_time.asc())
        .load(db)?)

}

#[get("/season/<id>/qualifiers")]
async fn qualifiers(id: i32, mut db: ConnectionWrapper<'_>) -> Json<Result<Vec<Qualifier>, ApiError>> {
    Json(get_qualifiers(id, &mut db))
}

pub fn build_rocket(rocket: Rocket<Build>) -> Rocket<Build> {
    rocket.mount("/api/v1",
        rocket::routes![
            qualifiers
        ]
    )
}