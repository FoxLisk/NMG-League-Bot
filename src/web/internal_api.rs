use crate::web::ConnectionWrapper;
use log::warn;
use nmg_league_bot::config::CONFIG;
use nmg_league_bot::models::bracket_race_infos::{BracketRaceInfo, BracketRaceInfoId};
use nmg_league_bot::utils::ResultErrToString;
use once_cell::sync::Lazy;
use regex::Regex;
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome};
use rocket::serde::json::Json;
use rocket::{Build, Request, Rocket, State};
use std::ops::DerefMut;
use tokio::sync::mpsc::Sender;

struct InternalAdmin {}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for InternalAdmin {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        static AUTHZ_REGEX: Lazy<Result<Regex, regex::Error>> =
            Lazy::new(|| Regex::new(r"^ApiKey\s+(?<key>\w+)$"));
        let re = match &*AUTHZ_REGEX {
            Ok(re) => re,
            Err(e) => {
                warn!("Error with AUTHZ_REGEX, unable to authorize for internal API: {e}");

                return Outcome::Failure((Status::InternalServerError, ()));
            }
        };
        let az = match request.headers().get_one("Authorization") {
            Some(az) => az,
            None => {
                return Outcome::Failure((Status::Unauthorized, ()));
            }
        };
        if let Some(provided_token) = re
            .captures(az)
            .and_then(|c| c.name("key"))
            .map(|m| m.as_str())
        {
            if provided_token == CONFIG.internal_api_secret {
                return Outcome::Success(Self {});
            }
        }
        Outcome::Failure((Status::Unauthorized, ()))
    }
}

#[rocket::post("/start_race_for/<bracket_race_info_id>")]
async fn start_race_for(
    _admin: InternalAdmin,
    bracket_race_info_id: i32,
    sender: &State<Sender<BracketRaceInfoId>>,
    mut conn: ConnectionWrapper<'_>,
) -> Json<Result<(), String>> {
    let res = match BracketRaceInfo::get_by_id(bracket_race_info_id, conn.deref_mut()) {
        Ok(bri) => sender.send(bri.get_id()).await.map_err_to_string(),
        Err(e) => Err(format!("Error getting BRI: {e}")),
    };
    Json(res)
}

pub fn build_rocket(rocket: Rocket<Build>, sender: Sender<BracketRaceInfoId>) -> Rocket<Build> {
    rocket
        .mount("/internal_api/v1", rocket::routes![start_race_for])
        .manage(sender)
}
