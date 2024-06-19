//! api lol. the idea is just stuff that returns json i guess

use crate::web::auth::Admin;
use crate::web::ConnectionWrapper;
use diesel::SqliteConnection;
use nmg_league_bot::models::qualifer_submission::QualifierSubmission;
use nmg_league_bot::utils::ResultErrToString;
use nmg_league_bot::ApiError;
use nmg_league_bot::NMGLeagueBotError;
use rocket::response::Responder;
use rocket::serde::json::Json;
use rocket::{delete, get, Build, Request, Rocket};
use serde::Serialize;

struct ApiResponse<T, E>(Result<T, E>);

impl<'r, 'o: 'r, T: Serialize, E: std::error::Error> Responder<'r, 'o> for ApiResponse<T, E> {
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'o> {
        Json(self.0.map_err_to_string()).respond_to(request)
    }
}

#[derive(diesel::Queryable, serde::Serialize)]
struct Qualifier {
    id: i32,
    player_name: String,
    time: i32,
    vod: String,
}

fn get_qualifiers(id: i32, db: &mut SqliteConnection) -> Result<Vec<Qualifier>, NMGLeagueBotError> {
    use crate::schema::{players, qualifier_submissions as qs};
    use diesel::prelude::*;
    Ok(qs::table
        .inner_join(players::table)
        .filter(qs::season_id.eq(id))
        .select((qs::id, players::name, qs::reported_time, qs::vod_link))
        .order_by(qs::reported_time.asc())
        .load(db)?)
}

// N.B. this one really should be season.id to make the SQL query simpler/faster
#[get("/season/<id>/qualifiers")]
async fn qualifiers(
    id: i32,
    mut db: ConnectionWrapper<'_>,
) -> ApiResponse<Vec<Qualifier>, NMGLeagueBotError> {
    ApiResponse(get_qualifiers(id, &mut db))
}

#[delete("/qualifiers/<id>")]
async fn delete_qualifier(
    id: i32,
    _admin: Admin,
    mut db: ConnectionWrapper<'_>,
) -> ApiResponse<(), NMGLeagueBotError> {
    let mut _delete_qualifier = || -> Result<(), NMGLeagueBotError> {
        let q = QualifierSubmission::get_by_id(id, &mut db)?;
        if q.safe_to_delete(&mut db)? {
            q.delete(&mut db)?;
        } else {
            // its weird that you put the `.into`() inside here!
            // Err(...).map_err(Into::into) works too
            return Err(ApiError::CannotDeletePastQualifiers.into());
        }
        Ok(())
    };
    ApiResponse(_delete_qualifier())
}

pub fn build_rocket(rocket: Rocket<Build>) -> Rocket<Build> {
    rocket.mount("/api/v1", rocket::routes![qualifiers, delete_qualifier])
}
