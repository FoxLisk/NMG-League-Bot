//! api lol. the idea is just stuff that returns json i guess

use std::ops::DerefMut;

use crate::web::auth::Admin;
use crate::web::ConnectionWrapper;
use diesel::SqliteConnection;
use log::debug;
use log::warn;
use nmg_league_bot::models::bracket_race_infos::BracketRaceInfo;
use nmg_league_bot::models::bracket_races::BracketRace;
use nmg_league_bot::models::bracket_races::Outcome;
use nmg_league_bot::models::bracket_races::PlayerResult;
use nmg_league_bot::models::bracket_rounds::BracketRound;
use nmg_league_bot::models::brackets::Bracket;
use nmg_league_bot::models::brackets::BracketState;
use nmg_league_bot::models::brackets::BracketType;
use nmg_league_bot::models::player::Player;
use nmg_league_bot::models::qualifer_submission::QualifierSubmission;
use nmg_league_bot::BracketRaceState;
use nmg_league_bot::NMGLeagueBotError;
use rocket::response::Responder;
use rocket::serde::json::Json;
use rocket::{delete, get, Build, Request, Rocket};
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("Cannot delete qualifiers that are already closed.")]
    CannotDeletePastQualifiers,

    #[error("Internal error")]
    NMGLeagueBotError(NMGLeagueBotError),

    #[error("Bad Request")]
    BadRequest,
}

// this is kinda cool, its like "passing through" the NMGLeagueBotError From implementations
impl<T> From<T> for ApiError
where
    NMGLeagueBotError: From<T>,
{
    fn from(value: T) -> Self {
        Self::NMGLeagueBotError(NMGLeagueBotError::from(value))
    }
}

struct ApiResponse<T>(Result<T, ApiError>);

impl<'r, 'o: 'r, T: Serialize> Responder<'r, 'o> for ApiResponse<T> {
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'o> {
        // is logging in here kosher?
        let returnable = match self.0 {
            Ok(val) => Json(Ok(val)),
            Err(e) => {
                // we return a generic error so we want to log the actual error
                warn!("Error fulfilling API request: {e:?}");
                Json(Err(e.to_string()))
            }
        };
        returnable.respond_to(request)
    }
}

#[derive(Serialize, Deserialize)]
struct ApiBracket {
    id: i32,
    name: String,
    season_id: i32,
    state: BracketState,
    bracket_type: BracketType,
}

impl TryFrom<Bracket> for ApiBracket {
    type Error = serde_json::Error;

    fn try_from(value: Bracket) -> Result<Self, Self::Error> {
        let state = value.state()?;
        let bracket_type = value.bracket_type()?;
        Ok(Self {
            id: value.id,
            name: value.name,
            season_id: value.season_id,
            state,
            bracket_type,
        })
    }
}

#[derive(diesel::Queryable, Serialize, Deserialize)]
struct ApiQualifier {
    id: i32,
    player_id: i32,
    player_name: String,
    time: i32,
    vod: String,
}

#[derive(Serialize, Deserialize)]
struct ApiRace {
    // race
    pub id: i32,
    pub bracket_id: i32,
    pub round: i32,
    pub player_1_id: i32,
    pub player_2_id: i32,
    pub state: BracketRaceState,
    pub player_1_result: Option<PlayerResult>,
    pub player_2_result: Option<PlayerResult>,
    pub outcome: Option<Outcome>,
    // race info
    pub scheduled_for: Option<i64>,
    pub racetime_gg_url: Option<String>,
    pub restream_channel: Option<String>,
}

impl TryFrom<(BracketRace, BracketRaceInfo, BracketRound)> for ApiRace {
    type Error = serde_json::Error;

    fn try_from(value: (BracketRace, BracketRaceInfo, BracketRound)) -> Result<Self, Self::Error> {
        let (race, info, round) = value;
        let player_1_result = race.player_1_result().transpose()?;
        let player_2_result = race.player_2_result().transpose()?;
        let state = race.state()?;
        let outcome = race.outcome()?;
        Ok(Self {
            id: race.id,
            bracket_id: race.bracket_id,
            round: round.round_num,
            player_1_id: race.player_1_id,
            player_2_id: race.player_2_id,
            state,
            player_1_result,
            player_2_result,
            outcome,
            scheduled_for: info.scheduled_for,
            racetime_gg_url: info.racetime_gg_url,
            restream_channel: info.restream_channel,
        })
    }
}

fn get_qualifiers(ordinal: i32, db: &mut SqliteConnection) -> Result<Vec<ApiQualifier>, ApiError> {
    use crate::schema::{players, qualifier_submissions as qs, seasons};
    use diesel::prelude::*;
    Ok(qs::table
        .inner_join(seasons::table)
        .inner_join(players::table)
        .filter(seasons::ordinal.eq(ordinal))
        .select((
            qs::id,
            qs::player_id,
            players::name,
            qs::reported_time,
            qs::vod_link,
        ))
        .order_by(qs::reported_time.asc())
        .load(db)?)
}

#[get("/season/<ordinal>/qualifiers")]
async fn qualifiers(ordinal: i32, mut db: ConnectionWrapper<'_>) -> ApiResponse<Vec<ApiQualifier>> {
    ApiResponse(get_qualifiers(ordinal, &mut db))
}

#[delete("/qualifiers/<id>")]
async fn delete_qualifier(
    id: i32,
    _admin: Admin,
    mut db: ConnectionWrapper<'_>,
) -> ApiResponse<()> {
    let mut _delete_qualifier = || -> Result<(), ApiError> {
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

#[get("/players?<player_id>")]
async fn get_players(
    player_id: Vec<i32>,
    mut db: ConnectionWrapper<'_>,
) -> ApiResponse<Vec<Player>> {
    use diesel::prelude::*;
    use nmg_league_bot::schema::players;
    let res: Result<Vec<Player>, _> = if player_id.is_empty() {
        players::table.load(db.deref_mut())
    } else {
        players::table
            .filter(players::id.eq_any(player_id))
            .load(db.deref_mut())
    };

    ApiResponse(res.map_err(From::from))
}

fn db_objs_to_api_objs<DB, API>(db_objs: Vec<DB>) -> Result<Vec<API>, ApiError>
where
    API: TryFrom<DB>,
    // I couldn't figure this out on my own, but the compiler explained it.
    // The compiler actually gave `serde_json::Error: From<<API as TryFrom<DB>>::Error>`,
    // but we can be a bit more generic and accept any NMGLeagueBotError.
    NMGLeagueBotError: From<<API as TryFrom<DB>>::Error>,
{
    let collection = Vec::with_capacity(db_objs.len());
    let converted: Result<Vec<API>, ApiError> =
        db_objs.into_iter().try_fold(collection, |mut coll, b| {
            let api_bracket = API::try_from(b)?;
            coll.push(api_bracket);
            Ok(coll)
        });
    converted
}

#[get("/season/<ordinal>/brackets")]
async fn get_season_brackets(
    ordinal: i32,
    mut db: ConnectionWrapper<'_>,
) -> ApiResponse<Vec<ApiBracket>> {
    fn get_brackets(ordinal: i32, conn: &mut SqliteConnection) -> Result<Vec<Bracket>, ApiError> {
        use crate::schema::{brackets, seasons};
        use diesel::prelude::*;
        Ok(brackets::table
            .inner_join(seasons::table)
            .filter(seasons::ordinal.eq(ordinal))
            .select(brackets::all_columns)
            .load(conn)?)
    }

    ApiResponse(get_brackets(ordinal, &mut db).and_then(db_objs_to_api_objs))
}

#[get("/season/<ordinal>/races?<state>")]
async fn get_season_races(
    ordinal: i32,
    state: Option<String>,
    mut db: ConnectionWrapper<'_>,
) -> ApiResponse<Vec<ApiRace>> {
    let _get_races = |conn: &mut SqliteConnection| -> Result<Vec<(BracketRace, BracketRaceInfo, BracketRound)>, ApiError> {
        use crate::schema::{bracket_race_infos, bracket_races, bracket_rounds, brackets, seasons};
        use diesel::prelude::*;


        
        // let mut q = bracket_races::table
        // .inner_join(bracket_rounds::table)
        // .inner_join(brackets::table.inner_join(seasons::table)))
        //     .inner_join(
        //         bracket_races::table
        //             .inner_join(brackets::table.inner_join(seasons::table))
        //             .inner_join(bracket_rounds::table),
        //     )
        //     .select((
        //         bracket_races::all_columns,
        //         bracket_race_infos::all_columns,
        //         bracket_rounds::all_columns,
        //     ))
        //     .filter(seasons::ordinal.eq(ordinal))
        //     .into_boxed();

        let mut q = bracket_race_infos::table
            .inner_join(
                bracket_races::table
                    .inner_join(brackets::table.inner_join(seasons::table))
                    .inner_join(bracket_rounds::table),
            )
            .select((
                bracket_races::all_columns,
                bracket_race_infos::all_columns,
                bracket_rounds::all_columns,
            ))
            .filter(seasons::ordinal.eq(ordinal))
            .into_boxed();
        println!("Got state param: {state:?}");
        if let Some(state_inner) = state {
            match serde_json::from_str::<BracketRaceState>(&state_inner) {
                Ok(_) => {
                    // we make sure it is parseable, but we pass the unparsed value through
                    // because that's what's actually in the DB
                    q = q.filter(bracket_races::state.eq(state_inner));
                }
                Err(e) => {
                    debug!("Error parsing state param {state_inner}: {e}");
                    return Err(ApiError::BadRequest);
                }
            };
        }
        Ok(
            q.load::<(BracketRace, BracketRaceInfo, BracketRound)>(conn)?
        )
    };
    let data = _get_races(&mut db);

    ApiResponse(data.and_then(db_objs_to_api_objs))
}

pub fn build_rocket(rocket: Rocket<Build>) -> Rocket<Build> {
    rocket.mount(
        "/api/v1",
        rocket::routes![
            qualifiers,
            delete_qualifier,
            get_players,
            get_season_brackets,
            get_season_races
        ],
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use anyhow::anyhow;
    use bb8::Pool;
    use diesel::prelude::*;
    use diesel::SqliteConnection;
    use itertools::Itertools;
    use nmg_league_bot::models::bracket_races;
    use nmg_league_bot::models::bracket_races::BracketRace;
    use nmg_league_bot::models::bracket_races::NewBracketRace;
    use nmg_league_bot::models::bracket_rounds::NewBracketRound;
    use nmg_league_bot::models::brackets::Bracket;
    use nmg_league_bot::models::brackets::BracketType;
    use nmg_league_bot::models::brackets::NewBracket;
    use nmg_league_bot::models::player_bracket_entries::NewPlayerBracketEntry;
    use nmg_league_bot::models::season::NewSeason;
    use nmg_league_bot::{
        db::{run_migrations, DieselConnectionManager},
        models::{
            brackets::BracketState,
            player::{NewPlayer, Player},
        },
        schema::players,
        NMGLeagueBotError,
    };
    use reqwest::Url;
    use rocket::http::uri::Origin;
    use rocket::local::asynchronous::Client;

    use crate::web::api::ApiBracket;
    use crate::web::api::ApiRace;

    use super::build_rocket;

    /// this builds a rocket instance. it won't be a "full" rocket instance, the idea is to have a minimal one
    /// for testing just the API. this is not fully realistic, though so maybe that's a mistake...?
    async fn setup() -> anyhow::Result<Client> {
        // this runs migrations, so we need to run it even though we're not using the connection it builds
        let p: Pool<DieselConnectionManager> = Pool::builder()
            .max_size(1)
            .build(DieselConnectionManager::new_from_path(":memory:"))
            .await
            .unwrap();
        {
            let mut db = p.get().await?;
            run_migrations(&mut db)?;
        }
        let rocket = build_rocket(rocket::build()).manage(p);
        let client = Client::tracked(rocket).await?;
        Ok(client)
    }

    /// parses the result. The API returns objects that themselves are Result<T, String> objects. The outer
    /// result here is checking if we can parse the JSON. This should probably be used like:
    ///
    /// ```
    /// let parsed = parse_result::<Vec<Player>>(&body)?;
    /// ```
    ///
    /// which will cause `parsed` to be the API result already parsed into a Result<Vec<Player>, String>,
    /// and will immediately fail the test if the API somehow returns invalid JSON.
    fn parse_result<T>(body: &str) -> Result<Result<T, String>, serde_json::Error>
    where
        T: serde::de::DeserializeOwned,
    {
        serde_json::from_str::<Result<T, String>>(body)
    }

    #[test]
    fn test_json_errors() {
        let e = serde_json::from_str::<BracketState>(r#""blah""#)
            .err()
            .unwrap();
        assert!(e.is_data(), "{:?}", e.classify());
    }

    #[tokio::test]
    async fn test_rocket_builds() -> anyhow::Result<()> {
        setup().await?;
        Ok(())
    }

    #[tokio::test]
    async fn get_empty_players_result() -> anyhow::Result<()> {
        let c = setup().await?;
        let req = c.get("/api/v1/players");
        let resp = req.dispatch().await;
        assert_eq!(rocket::http::Status::Ok, resp.status(),);
        let body = resp
            .into_string()
            .await
            .ok_or(anyhow!("Failed to load body"))?;
        let parsed = parse_result::<Vec<Player>>(&body)?;
        assert_eq!(Ok(Vec::<Player>::new()), parsed);
        Ok(())
    }

    async fn run_with_db<F, T>(c: &Client, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&mut SqliteConnection) -> anyhow::Result<T>,
    {
        let db = c.rocket().state::<Pool<DieselConnectionManager>>().unwrap();
        let mut conn = db.get().await?;
        f(&mut conn).map_err(From::from)
    }

    #[tokio::test]
    async fn test_get_one_player_result() -> anyhow::Result<()> {
        let c = setup().await?;
        let p = run_with_db(&c, |db| {
            NewPlayer::new("test", "1234", None, None)
                .save(db)
                .map_err(From::from)
        })
        .await?;
        let resp = c.get("/api/v1/players").dispatch().await;
        assert_eq!(rocket::http::Status::Ok, resp.status(),);
        let parsed = parse_result::<Vec<Player>>(&resp.into_string().await.unwrap())?;
        assert_eq!(Ok(vec![p]), parsed);
        Ok(())
    }

    #[tokio::test]
    async fn test_filter_players() -> anyhow::Result<()> {
        let c = setup().await?;
        let players = run_with_db(&c, |db| {
            NewPlayer::new("test", "1", None, None).save(db)?;
            NewPlayer::new("test2", "2", None, None).save(db)?;
            NewPlayer::new("test3", "3", None, None).save(db)?;
            players::table.load::<Player>(db).map_err(From::from)
        })
        .await?;

        let mut players_to_find: HashSet<Player> = HashSet::from_iter(players.clone());
        assert!(players_to_find.remove(&players[1]));
        let qs = players_to_find
            .iter()
            .map(|p| format!("player_id={}", p.id))
            .join("&");
        let url = format!("/api/v1/players?{qs}");
        // this is lazy, i should get IDs properly
        let resp = c.get(url).dispatch().await;

        assert_eq!(rocket::http::Status::Ok, resp.status(),);
        let parsed = parse_result::<Vec<Player>>(&resp.into_string().await.unwrap())?
            .map_err(|e| anyhow!("{e}"))?;
        assert_eq!(players_to_find, HashSet::from_iter(parsed.into_iter()));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_brackets() -> anyhow::Result<()> {
        let c = setup().await?;
        let s = run_with_db(&c, |db| {
            let ns = NewSeason::new("Any% NMG", "alttp", "Any% NMG", db)?.save(db)?;
            NewBracket::new(&ns, "bracket 1", BracketType::Swiss).save(db)?;
            NewBracket::new(&ns, "bracket 2", BracketType::RoundRobin).save(db)?;
            Ok(ns)
        })
        .await?;
        let resp = c
            .get(format!("/api/v1/season/{}/brackets", s.ordinal))
            .dispatch()
            .await;

        assert_eq!(rocket::http::Status::Ok, resp.status(),);
        let parsed = parse_result::<Vec<ApiBracket>>(&resp.into_string().await.unwrap())?
            .map_err(|e| anyhow!("{e}"))?;
        assert_eq!(2, parsed.len());
        Ok(())
    }
    #[tokio::test]
    async fn test_get_races() -> anyhow::Result<()> {
        let c = setup().await?;
        let s = run_with_db(&c, |db| {
            let ns = NewSeason::new("Any% NMG", "alttp", "Any% NMG", db)?.save(db)?;
            let b = NewBracket::new(&ns, "bracket 1", BracketType::Swiss).save(db)?;
            let round = NewBracketRound::new(&b, 1).save(db)?;
            let p1 = NewPlayer::new("p1", "1", None, None).save(db)?;
            let p2 = NewPlayer::new("p2", "2", None, None).save(db)?;
            NewPlayerBracketEntry::new(&b, &p1).save(db)?;
            NewPlayerBracketEntry::new(&b, &p2).save(db)?;
            bracket_races::insert_bulk(&vec![NewBracketRace::new(&b, &round, &p1, &p2)], db)?;
            let br = BracketRace::get_by_id(1, db)?;
            // hydrate the BRI
            br.info(db)?;
            assert_eq!(1, b.bracket_races(db)?.len());
            Ok(ns)
        })
        .await?;

        
        let unfiltered_resp = c
            .get(format!("/api/v1/season/{}/races", s.ordinal))
            .dispatch()
            .await;
        assert_eq!(rocket::http::Status::Ok, unfiltered_resp.status(),);
        let parsed = parse_result::<Vec<ApiRace>>(&unfiltered_resp.into_string().await.unwrap())?
            .map_err(|e| anyhow!("{e}"))?;
        assert_eq!(1, parsed.len());

        
        let bad_state = c
            .get(format!("/api/v1/season/{}/races?state=foo", s.ordinal))
            .dispatch()
            .await;
        assert_eq!(rocket::http::Status::Ok, bad_state.status(),);
        let parsed = parse_result::<Vec<ApiRace>>(&bad_state.into_string().await.unwrap())?;
        assert!(parsed.is_err());
        assert_eq!("Bad Request", parsed.err().unwrap());

        // `urlencoding::encode()` urlencodes the `=` sign!
        let new_resp = c
            .get(format!("/api/v1/season/{}/races?state={}", s.ordinal, urlencoding::encode(r#""New""#)))
            .dispatch()
            .await;
        assert_eq!(rocket::http::Status::Ok, new_resp.status(),);
        let parsed = parse_result::<Vec<ApiRace>>(&new_resp.into_string().await.unwrap())?
            .map_err(|e| anyhow!("{e}"))?;
        assert_eq!(1, parsed.len());

        let scheduled_resp = c
            .get(format!(
                "/api/v1/season/{}/races?state={}",
                s.ordinal,
                urlencoding::encode(r#""Scheduled""#)
            ))
            .dispatch()
            .await;
        assert_eq!(rocket::http::Status::Ok, scheduled_resp.status(),);
        let parsed = parse_result::<Vec<ApiRace>>(&scheduled_resp.into_string().await.unwrap())?
            .map_err(|e| anyhow!("{e}"))?;
        assert_eq!(0, parsed.len());
        Ok(())
    }
}
