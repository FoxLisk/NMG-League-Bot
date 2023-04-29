//! api lol. the idea is just stuff that returns json i guess

use crate::web::ConnectionWrapper;
use diesel::result::Error;
use diesel::SqliteConnection;
use rocket::serde::json::Json;
use rocket::{get, Build, Rocket};

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
    fn from(_value: Error) -> Self {
        Self::DatabaseError
    }
}

fn get_qualifiers(id: i32, db: &mut SqliteConnection) -> Result<Vec<Qualifier>, ApiError> {
    use crate::schema::{players, qualifier_submissions as qs};
    use diesel::prelude::*;
    Ok(qs::table
        .inner_join(players::table)
        .filter(qs::season_id.eq(id))
        .select((players::name, qs::reported_time, qs::vod_link))
        .order_by(qs::reported_time.asc())
        .load(db)?)
}

#[get("/season/<id>/qualifiers")]
async fn qualifiers(
    id: i32,
    mut db: ConnectionWrapper<'_>,
) -> Json<Result<Vec<Qualifier>, ApiError>> {
    Json(get_qualifiers(id, &mut db))
}

pub fn build_rocket(rocket: Rocket<Build>) -> Rocket<Build> {
    rocket.mount("/api/v1", rocket::routes![qualifiers])
}
