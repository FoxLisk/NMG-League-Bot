use itertools::Itertools;
use nmg_league_bot::{BracketRaceState, NMGLeagueBotError};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rocket::http::Status;
use rocket::{get, Request, State};

use rocket_dyn_templates::{context, Template};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::Duration;

use crate::discord::discord_state::DiscordOperations;
use crate::discord::discord_state::DiscordState;
use crate::schema;
use crate::shutdown::Shutdown;
use crate::web::auth::{Admin, OauthClient};
use crate::web::session_manager::SessionManager as _SessionManager;
use bb8::{Pool, PooledConnection};
use diesel::prelude::*;
use log::{debug, info, warn};
use nmg_league_bot::db::{get_diesel_pool, DieselConnectionManager};
use nmg_league_bot::models::asyncs::race::{AsyncRace, RaceState};
use nmg_league_bot::models::asyncs::race_run::{AsyncRaceRun, RaceRunState};
use nmg_league_bot::models::bracket_race_infos::{BracketRaceInfo, BracketRaceInfoId};
use nmg_league_bot::models::bracket_races::{BracketRace, PlayerResult};
use nmg_league_bot::models::bracket_rounds::BracketRound;
use nmg_league_bot::models::brackets::{Bracket, BracketError, BracketType};
use nmg_league_bot::models::player::Player;
use nmg_league_bot::models::season::Season;
use nmg_league_bot::utils::format_hms;
use rocket::request::{FromRequest, Outcome};
use rocket::response::Redirect;
use rocket_dyn_templates::tera::{to_value, try_get_value, Value};
use serde::Serialize;
use std::ops::{Deref, DerefMut};
use tokio::sync::mpsc::Sender;
use twilight_model::id::marker::UserMarker;
use twilight_model::id::Id;
mod api;
mod auth;
mod internal_api;
mod session_manager;
mod statics;

type SessionManager = _SessionManager<Id<UserMarker>>;

const SESSION_COOKIE_NAME: &str = "nmg_league_session";
const DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

#[macro_export]
macro_rules! uri {
      ($($t:tt)*) => (rocket::uri!("/", crate::web:: $($t)*))
}

struct ConnectionWrapper<'a>(PooledConnection<'a, DieselConnectionManager>);

impl<'a> Deref for ConnectionWrapper<'a> {
    type Target = SqliteConnection;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> DerefMut for ConnectionWrapper<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for ConnectionWrapper<'r> {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let pool = match request
            .guard::<&State<Pool<DieselConnectionManager>>>()
            .await
        {
            Outcome::Success(pool) => pool,
            _ => {
                return Outcome::Failure((Status::InternalServerError, ()));
            }
        };
        match pool.get().await {
            Ok(a) => Outcome::Success(ConnectionWrapper(a)),
            Err(_) => Outcome::Failure((Status::InternalServerError, ())),
        }
    }
}

#[derive(Serialize, Debug, Default)]
struct BaseContext {
    current_season: Option<Season>,
    admin: bool,
}

impl BaseContext {
    fn new(conn: &mut SqliteConnection, admin: &Option<Admin>) -> Self {
        let current_season = Season::get_active_season(conn).ok().flatten();
        Self {
            current_season,
            admin: admin.is_some(),
        }
    }
}

// N.B. this should either not be called DiscordState, or we should manage a separate
// connection pool for the website
#[get("/asyncs")]
async fn async_view(admin: Admin, discord_state: &State<Arc<DiscordState>>) -> Template {
    let mut cxn = match discord_state.diesel_cxn().await {
        Ok(c) => c,
        Err(e) => {
            return Template::render("asyncs", Context::error(e.to_string()));
        }
    };
    let query = schema::races::table.inner_join(schema::race_runs::table);
    let results: Vec<(AsyncRace, AsyncRaceRun)> =
        match query.load::<(AsyncRace, AsyncRaceRun)>(cxn.deref_mut()) {
            Ok(r) => r,
            Err(e) => {
                return Template::render("asyncs", Context::error(e.to_string()));
            }
        };

    #[derive(Serialize, Default)]
    struct Context {
        base_context: BaseContext,
        error: Option<String>,
        finished: Vec<ViewRace>,
        created: Vec<ViewRace>,
        abandoned: Vec<ViewRace>,
        cancelled: Vec<ViewRace>,
    }

    impl Context {
        fn error(s: String) -> Self {
            Self {
                error: Some(s),
                ..Default::default()
            }
        }
    }

    #[derive(Serialize)]
    struct ViewRaceRun {
        run_uuid: String,
        racer_name: String,
        filenames: String,
        run_state: RaceRunState,
        vod: Option<String>,
        started: Option<String>,
        bot_time_to_finish: Option<String>,
        user_reported_time: Option<String>,
        time_from_finish_to_report: Option<String>,
    }

    struct ViewRaceBuilder {
        id: i32,
        state: RaceState,
        on_start_message: Option<String>,
        p1: Option<ViewRaceRun>,
        p2: Option<ViewRaceRun>,
    }

    #[derive(Serialize)]
    struct ViewRace {
        id: i32,
        state: RaceState,
        on_start_message: Option<String>,
        p1: ViewRaceRun,
        p2: ViewRaceRun,
    }

    impl ViewRaceBuilder {
        fn from_race(r: AsyncRace) -> Self {
            Self {
                id: r.id,
                state: r.state,
                on_start_message: r.on_start_message,
                p1: None,
                p2: None,
            }
        }

        fn add_run(&mut self, vrr: ViewRaceRun) -> Result<(), ()> {
            if self.p1.is_none() {
                self.p1 = Some(vrr);
                Ok(())
            } else if self.p2.is_none() {
                self.p2 = Some(vrr);
                Ok(())
            } else {
                Err(())
            }
        }

        fn build(self) -> Result<ViewRace, ()> {
            // does this do a bunch of memory copying?
            // it doesn't matter but i am kind of curious
            let p1 = self.p1.ok_or(())?;
            let p2 = self.p2.ok_or(())?;
            if p1.run_uuid.lt(&p2.run_uuid) {
                Ok(ViewRace {
                    id: self.id,
                    state: self.state,
                    on_start_message: self.on_start_message,
                    p1,
                    p2,
                })
            } else {
                Ok(ViewRace {
                    id: self.id,
                    state: self.state,
                    on_start_message: self.on_start_message,
                    p1: p2,
                    p2: p1,
                })
            }
        }
    }

    let mut race_builders: HashMap<i32, ViewRaceBuilder> = Default::default();

    for (race, run) in results {
        let vr = race_builders
            .entry(race.id)
            .or_insert(ViewRaceBuilder::from_race(race));
        let username = match run.racer_id() {
            Ok(uid) => discord_state
                .get_user(uid)
                .map(|u| u.global_name.unwrap_or(u.name))
                .unwrap_or("Unknown".to_string()),
            Err(e) => {
                warn!("Error parsing racer id {}", e);
                "Unknown".to_string()
            }
        };
        let started = run
            .get_started_at()
            .map(|s| s.format(DATETIME_FORMAT).to_string());
        let bot_time_to_finish = run.get_time_to_finish();
        let time_from_finish_to_report = run.get_time_from_finish_to_report();
        let fns = run
            .filenames()
            .map(|f| f.to_string())
            .unwrap_or("unknown error".to_string());
        let vrr = ViewRaceRun {
            run_uuid: run.uuid,
            racer_name: username,
            filenames: fns,
            run_state: run.state,
            vod: run.vod,
            started,
            bot_time_to_finish,
            user_reported_time: run.reported_run_time,
            time_from_finish_to_report,
        };
        vr.add_run(vrr).ok();
    }

    let mut finished = vec![];
    let mut created = vec![];
    let mut abandoned = vec![];
    let mut cancelled = vec![];

    for vrb in race_builders.into_values() {
        let vr = match vrb.build() {
            Ok(vr_) => vr_,
            Err(_) => {
                continue;
            }
        };
        let destination = match vr.state {
            RaceState::CREATED => &mut created,
            RaceState::FINISHED => &mut finished,
            RaceState::ABANDONED => &mut abandoned,
            RaceState::CANCELLED_BY_ADMIN => &mut cancelled,
        };
        destination.push(vr);
    }

    finished.sort_by(|a, b| a.id.cmp(&b.id));
    created.sort_by(|a, b| a.id.cmp(&b.id));
    abandoned.sort_by(|a, b| a.id.cmp(&b.id));
    cancelled.sort_by(|a, b| a.id.cmp(&b.id));

    Template::render(
        "asyncs",
        Context {
            base_context: BaseContext::new(cxn.deref_mut(), &Some(admin)),
            error: None,
            finished,
            created,
            abandoned,
            cancelled,
        },
    )
}

#[derive(Serialize)]
struct DisplayPlayer {
    name: String,
    status: String,
    winner: bool,
    loser: bool,
}

#[derive(Serialize)]
struct DisplayRace {
    race_id: i32,
    player_1: DisplayPlayer,
    player_2: DisplayPlayer,
    scheduled: Option<String>,
    channel: Option<String>,
}

impl DisplayPlayer {
    fn new(
        p: &Player,
        res: Option<Result<PlayerResult, serde_json::Error>>,
        winner: bool,
        loser: bool,
    ) -> Self {
        let status = if let Some(Ok(r)) = res {
            format!("{r}")
        } else {
            "".to_string()
        };
        Self {
            name: p.name.clone(),
            status,
            winner,
            loser,
        }
    }
}

impl DisplayRace {
    fn new(p1: &Player, p2: &Player, race: &BracketRace, race_info: &BracketRaceInfo) -> Self {
        use nmg_league_bot::models::bracket_races::Outcome::{P1Win, P2Win};
        let outcome = race.outcome().unwrap_or(None);
        let player_1 = DisplayPlayer::new(
            p1,
            race.player_1_result(),
            outcome == Some(P1Win),
            outcome == Some(P2Win),
        );
        let player_2 = DisplayPlayer::new(
            p2,
            race.player_2_result(),
            outcome == Some(P2Win),
            outcome == Some(P1Win),
        );

        let (scheduled, channel) = match outcome {
            Some(_) => (None, None),
            None => {
                let scheduled = if let Some(utc_dt) = race_info.scheduled() {
                    Some(
                        utc_dt
                            .with_timezone(&chrono_tz::US::Eastern)
                            .format("%A, %B %d at %_I:%M %p (%Z)")
                            .to_string(),
                    )
                } else {
                    None
                };
                (scheduled, race_info.restream_channel.clone())
            }
        };
        Self {
            race_id: race.id,
            player_1,
            player_2,
            scheduled,
            channel,
        }
    }
}

#[derive(Serialize)]
struct DisplayRound {
    round_num: i32,
    races: Vec<DisplayRace>,
}

#[derive(Serialize)]
struct DisplayBracket {
    bracket: Bracket,
    /// all the rounds of the bracket, in ascending order (i.e. round 1 first, round 2 second)
    rounds: Vec<DisplayRound>,
    is_round_robin: bool,
}

#[derive(Serialize)]
struct BracketsContext {
    season: Season,
    brackets: Vec<DisplayBracket>,
    base_context: BaseContext,
}

fn get_display_bracket(
    bracket: Bracket,
    conn: &mut SqliteConnection,
) -> Result<DisplayBracket, NMGLeagueBotError> {
    let races = bracket.bracket_races(conn)?;
    let rounds_by_id: HashMap<i32, BracketRound> =
        HashMap::from_iter(bracket.rounds(conn)?.into_iter().map(|r| (r.id, r)));
    let players_by_id: HashMap<i32, Player> =
        HashMap::from_iter(bracket.players(conn)?.into_iter().map(|r| (r.id, r)));

    let mut display_rounds_by_num: HashMap<i32, DisplayRound> = Default::default();
    for race in races {
        let round = match rounds_by_id.get(&race.round_id) {
            Some(r) => r,
            None => {
                info!("Missing round with id {}", race.round_id);
                continue;
            }
        };
        let p1 = match players_by_id.get(&race.player_1_id) {
            Some(p) => p,
            None => {
                continue;
            }
        };

        let p2 = match players_by_id.get(&race.player_2_id) {
            Some(p) => p,
            None => {
                continue;
            }
        };
        let r = race.info(conn)?;
        let dr = DisplayRace::new(p1, p2, &race, &r);

        display_rounds_by_num
            .entry(round.round_num)
            .or_insert(DisplayRound {
                round_num: round.round_num,
                races: vec![],
            })
            .races
            .push(dr);
    }
    let rounds = display_rounds_by_num
        .into_iter()
        .sorted_by_key(|(n, _rs)| n.clone())
        .map(|(_n, rs)| rs)
        .collect();
    let is_round_robin = bracket.bracket_type()? == BracketType::RoundRobin;
    Ok(DisplayBracket {
        bracket,
        rounds,
        is_round_robin,
    })
}

#[get("/season/<season_ordinal>/bracket/<bracket_id>")]
async fn bracket_detail(
    season_ordinal: i32,
    bracket_id: i32,
    admin: Option<Admin>,
    mut db: ConnectionWrapper<'_>,
) -> Result<Template, Status> {
    let szn = match Season::get_by_ordinal(season_ordinal, &mut db) {
        Ok(s) => Ok(s),
        Err(diesel::result::Error::NotFound) => Err(Status::NotFound),

        Err(_) => Err(Status::InternalServerError),
    }?;
    let bracket = match Bracket::get_by_id(bracket_id, &mut db) {
        Ok(s) => Ok(s),
        Err(diesel::result::Error::NotFound) => Err(Status::NotFound),

        Err(_) => Err(Status::InternalServerError),
    }?;
    if bracket.season_id != szn.id {
        return Err(Status::NotFound);
    }
    let disp_b = get_display_bracket(bracket, &mut db).or(Err(Status::InternalServerError))?;
    let ctx = context! {
        season: szn,
        bracket: disp_b,
        base_context: BaseContext::new(&mut db, &admin)
    };

    Ok(Template::render("bracket_detail", ctx))
}

#[get("/season/<season_ordinal>/brackets")]
async fn season_brackets(
    season_ordinal: i32,
    mut db: ConnectionWrapper<'_>,
    admin: Option<Admin>,
) -> Result<Template, Status> {
    let szn = match Season::get_by_ordinal(season_ordinal, &mut db) {
        Ok(s) => Ok(s),
        Err(diesel::result::Error::NotFound) => Err(Status::NotFound),

        Err(_) => Err(Status::InternalServerError),
    }?;
    let brackets = match szn.brackets(&mut db) {
        Ok(b) => b,
        Err(e) => {
            warn!("Error getting season brackets: {e}");
            return Err(Status::InternalServerError);
        }
    };
    #[derive(Serialize)]
    struct BracketInfo {
        id: i32,
        name: String,
        url: String,
    }
    let infos = brackets
        .into_iter()
        .map(|b| BracketInfo {
            id: b.id,
            name: b.name,
            url: format!("/season/{}/bracket/{}", szn.ordinal, b.id),
        })
        .collect::<Vec<_>>();
    let ctx = context! {
        season: szn,
        brackets: infos,
        base_context: BaseContext::new(&mut db, &admin)
    };

    Ok(Template::render("season_brackets", ctx))
}

#[derive(Serialize)]
struct StandingsPlayer {
    name: String,
    player_detail_url: String,
    points: f32,
    opponent_points: f32,
    average_time_adjusted: String,
    average_time_finished: String,
}

#[derive(Serialize)]
struct StandingsBracket {
    name: String,
    players: Vec<StandingsPlayer>,
}

#[derive(Serialize)]
struct StandingsContext {
    season: Season,
    brackets: Vec<StandingsBracket>,
    base_context: BaseContext,
}

fn get_standings_context(
    szn: Season,
    admin: &Option<Admin>,
    conn: &mut SqliteConnection,
) -> Result<StandingsContext, diesel::result::Error> {
    let mut ctx_brackets = vec![];

    for bracket in szn.brackets(conn)? {
        let mut players = bracket.players(conn)?;
        let standings = match bracket.standings(conn) {
            Ok(s) => s,
            Err(BracketError::DBError(e)) => {
                return Err(e);
            }
            Err(BracketError::InvalidState) => {
                vec![]
            }
            Err(e) => {
                warn!("Error getting standings for bracket {bracket:?}: {e:?}");
                continue;
            }
        };
        let sps: Vec<StandingsPlayer> = if standings.is_empty() {
            players.sort_by_key(|p| p.id);
            players
                .into_iter()
                .map(|p| StandingsPlayer {
                    player_detail_url: uri!(player_detail(name = &p.name)).to_string(),
                    name: p.name,
                    points: 0.0,
                    opponent_points: 0.0,
                    average_time_adjusted: "".to_string(),
                    average_time_finished: "".to_string(),
                })
                .collect()
        } else {
            let players_map: HashMap<i32, String> =
                players.into_iter().map(|p| (p.id, p.name)).collect();
            standings
                .iter()
                .map(|s| {
                    // N.B. it is probably more correct to do `players.remove` instead of `players.get.cloned`
                    let name = players_map
                        .get(&s.id)
                        .cloned()
                        .unwrap_or("Unknown".to_string());

                    let avg = if s.avg_time_finished() > 0.0 {
                        format_hms(s.avg_time_finished() as u64)
                    } else {
                        "N/A".to_string()
                    };
                    StandingsPlayer {
                        player_detail_url: uri!(player_detail(name = &name)).to_string(),
                        name,
                        points: (s.points as f32) / 2.0,
                        opponent_points: (s.opponent_points as f32) / 2.0,
                        average_time_adjusted: format_hms(s.avg_time_adjusted() as u64),
                        average_time_finished: avg,
                    }
                })
                .collect()
        };
        ctx_brackets.push(StandingsBracket {
            name: bracket.name,
            players: sps,
        });
    }
    Ok(StandingsContext {
        season: szn,
        brackets: ctx_brackets,
        base_context: BaseContext::new(conn, admin),
    })
}

#[get("/season/<season_ordinal>/standings")]
async fn season_standings(
    season_ordinal: i32,
    admin: Option<Admin>,
    mut db: ConnectionWrapper<'_>,
) -> Result<Template, Status> {
    let szn = match Season::get_by_ordinal(season_ordinal, &mut db) {
        Ok(s) => Ok(s),
        Err(diesel::result::Error::NotFound) => Err(Status::NotFound),
        Err(_) => Err(Status::InternalServerError),
    }?;

    let ctx =
        get_standings_context(szn, &admin, &mut db).map_err(|_| Status::InternalServerError)?;
    Ok(Template::render("season_standings", ctx))
}

#[get("/season/<season_ordinal>/qualifiers")]
async fn season_qualifiers(
    season_ordinal: i32,
    mut db: ConnectionWrapper<'_>,
    admin: Option<Admin>,
) -> Result<Template, Status> {
    let szn = match Season::get_by_ordinal(season_ordinal, &mut db) {
        Ok(s) => Ok(s),
        Err(diesel::result::Error::NotFound) => Err(Status::NotFound),
        Err(_) => Err(Status::InternalServerError),
    }?;
    let base_context = BaseContext::new(&mut db, &admin);
    Ok(Template::render(
        "season_qualifiers",
        context!(season: szn, base_context),
    ))
}

#[get("/season/<season_ordinal>")]
async fn season_redirect(season_ordinal: i32) -> Redirect {
    Redirect::to(uri!(season_brackets(season_ordinal = season_ordinal)))
}

#[get("/seasons")]
async fn season_history(
    mut db: ConnectionWrapper<'_>,
    admin: Option<Admin>,
) -> Result<Template, Status> {
    let ctx = context! {
        base_context: BaseContext::new(&mut db, &admin),
    };
    Ok(Template::render("season_history", ctx))
}

async fn render_player_detail<F>(
    func: F,
    admin: Option<Admin>,
    state: &Arc<DiscordState>,
    db: &mut SqliteConnection,
) -> Result<Template, Status>
where
    F: FnOnce(&mut SqliteConnection) -> Result<Option<Player>, diesel::result::Error>,
{
    let bc = BaseContext::new(db, &admin);
    let player = match func(db) {
        Ok(Some(p)) => p,
        Ok(None) => {
            return Ok(Template::render(
                "player_detail_empty",
                context! {base_context: bc },
            ));
        }
        Err(e) => {
            warn!("Error getting player info: {e}");
            return Err(Status::InternalServerError);
        }
    };

    #[derive(Debug, Serialize)]
    struct RaceHistory {
        opponent: String,
        round: i32,
        /// "win", "loss", or "draw"
        outcome: &'static str,
        time: String,
        opponent_time: String,
    }

    #[derive(Debug, Serialize)]
    struct SeasonInfo {
        url: String,
        title: String,
        season_ordinal: i32,
    }

    #[derive(Debug, Serialize)]
    struct SeasonHistory {
        season: SeasonInfo,
        races: Vec<RaceHistory>,
    }
    impl SeasonHistory {
        fn new(season: Season, bracket: Bracket) -> Self {
            let title = format!(
                "Season {}: {} ({})",
                season.ordinal, season.format, bracket.name
            );
            let url = uri!(bracket_detail(
                season_ordinal = season.ordinal,
                bracket_id = bracket.id
            ))
            .to_string();
            Self {
                season: SeasonInfo {
                    title,
                    url,
                    season_ordinal: season.ordinal,
                },
                races: Default::default(),
            }
        }
        fn add_race(
            &mut self,
            race: BracketRace,
            round: BracketRound,
            our_player_id: i32,
            players: &HashMap<i32, Player>,
        ) {
            // skip unfinished races
            match race.state() {
                Ok(s) => match s {
                    BracketRaceState::Finished => {}
                    _ => {
                        debug!("Race {} not finished, skipping.", race.id);
                    }
                },
                Err(e) => {
                    warn!("Error fetching race state. Skipping. {e}");
                    return;
                }
            }
            let we_are_p1 = race.player_1_id == our_player_id;
            // writing code like this makes me think I have made a terrible mistake somewhere
            let outcome = match race.outcome() {
                Ok(Some(o)) => match o {
                    nmg_league_bot::models::bracket_races::Outcome::Tie => "draw",
                    nmg_league_bot::models::bracket_races::Outcome::P1Win => {
                        if we_are_p1 {
                            "win"
                        } else {
                            "loss"
                        }
                    }
                    nmg_league_bot::models::bracket_races::Outcome::P2Win => {
                        if we_are_p1 {
                            "loss"
                        } else {
                            "win"
                        }
                    }
                },
                Ok(None) => {
                    info!("Race {} not finished, skipping", race.id);
                    return;
                }
                Err(e) => {
                    warn!("Error fetching race outcome, skipping: {e}");
                    return;
                }
            };
            let (our_result, their_result, their_id) = if we_are_p1 {
                (
                    race.player_1_result(),
                    race.player_2_result(),
                    race.player_2_id,
                )
            } else {
                (
                    race.player_2_result(),
                    race.player_1_result(),
                    race.player_1_id,
                )
            };
            fn res_opt_to_display(r: Option<Result<PlayerResult, serde_json::Error>>) -> String {
                match r {
                    Some(Ok(pr)) => pr.to_string(),
                    _ => "Missing data".to_string(),
                }
            }

            let p = players
                .get(&their_id)
                .map(|p| p.name.clone())
                .unwrap_or("Unknown".to_string());

            let rh = RaceHistory {
                opponent: p,
                round: round.round_num,
                outcome: outcome,
                time: res_opt_to_display(our_result),
                opponent_time: res_opt_to_display(their_result),
            };
            self.races.push(rh);
            self.races.sort_by_key(|rh| rh.round);
        }
    }
    #[derive(Debug, Serialize)]
    struct PlayerHistory {
        seasons: Vec<SeasonHistory>,
    }

    fn get_player_history(
        db: &mut SqliteConnection,
        id: i32,
    ) -> Result<PlayerHistory, diesel::result::Error> {
        use crate::schema::bracket_races;
        use diesel::prelude::*;
        let results: Vec<(BracketRace, (BracketRound, (Bracket, Season)))> = bracket_races::table
            .filter(
                bracket_races::player_1_id
                    .eq(id)
                    .or(bracket_races::player_2_id.eq(id)),
            )
            .inner_join(crate::schema::bracket_rounds::table.inner_join(
                crate::schema::brackets::table.inner_join(crate::schema::seasons::table),
            ))
            .load(db)?;
        let all_player_ids = results
            .iter()
            .map(|(r, _)| vec![r.player_1_id, r.player_2_id])
            .flatten()
            .collect::<HashSet<_>>();
        let players: Vec<Player> = crate::schema::players::table
            .filter(crate::schema::players::id.eq_any(all_player_ids))
            .load(db)?;
        let player_map: HashMap<i32, Player> = players.into_iter().map(|p| (p.id, p)).collect();
        let mut season_histories: HashMap<i32, SeasonHistory> = Default::default();

        for (race, (round, (bracket, season))) in results.into_iter() {
            let history = season_histories
                .entry(season.id)
                .or_insert(SeasonHistory::new(season, bracket));

            history.add_race(race, round, id, &player_map);
        }
        let mut histories: Vec<SeasonHistory> = season_histories.into_values().collect();
        histories.sort_by_key(|s| s.season.season_ordinal);
        Ok(PlayerHistory { seasons: histories })
    }

    let player_history = match get_player_history(db, player.id) {
        Ok(p) => Some(p),
        Err(e) => {
            warn!("Error getting player season history: {e}");
            None
        }
    };
    let pfp = match state.get_player_pfp(&player).await {
        Ok(s) => s,
        Err(e) => {
            warn!("Error fetching pfp: {e}");
            None
        }
    };

    let ctx = context! {
        base_context: bc,
        player,
        player_history,
        pfp
    };
    Ok(Template::render("player_detail", ctx))
}

// TODO: having both a State (that could get a diesel cxn) and a db straight from the request guard
//       is pretty bad form
#[get("/player/<id>")]
async fn player_detail_by_id(
    id: i32,
    admin: Option<Admin>,
    state: &State<Arc<DiscordState>>,
    mut db: ConnectionWrapper<'_>,
) -> Result<Template, Status> {
    render_player_detail(|db| Player::get_by_id(id, db), admin, state, db.deref_mut()).await
}

#[get("/player/<name>", rank = 2)]
async fn player_detail(
    name: String,
    admin: Option<Admin>,
    state: &State<Arc<DiscordState>>,
    mut db: ConnectionWrapper<'_>,
) -> Result<Template, Status> {
    render_player_detail(
        |db| Player::get_by_name(&name, db),
        admin,
        state,
        db.deref_mut(),
    )
    .await
}

#[get("/helper_bot")]
async fn helper_bot(
    mut db: ConnectionWrapper<'_>,
    admin: Option<Admin>,
) -> Result<Template, Status> {
    Ok(Template::render(
        "helper_bot",
        context! { base_context: BaseContext::new(&mut db, &admin)},
    ))
}
#[get("/")]
async fn home(mut db: ConnectionWrapper<'_>, admin: Option<Admin>) -> Result<Template, Status> {
    Ok(Template::render(
        "home",
        context! { base_context: BaseContext::new(&mut db, &admin)},
    ))
}

/// this function is for use in templates
fn option_default(
    v: &Value,
    h: &HashMap<String, Value>,
) -> rocket_dyn_templates::tera::Result<Value> {
    let d = match h.get("default") {
        None => {
            return Err(rocket_dyn_templates::tera::Error::msg(
                "option_default missing required argument `default`",
            ));
        }
        Some(d) => {
            try_get_value!("default", "option_default", String, d)
        }
    };
    match v {
        Value::Null => Ok(to_value(d)?),

        _ => Ok(v.clone()),
    }
}

pub(crate) async fn launch_website(
    state: Arc<DiscordState>,
    bri_sender: Sender<BracketRaceInfoId>,
    mut shutdown: tokio::sync::broadcast::Receiver<Shutdown>,
) -> Result<(), rocket::error::Error> {
    let oauth_client = OauthClient::new();
    let session_manager: Arc<AsyncMutex<SessionManager>> =
        Arc::new(AsyncMutex::new(SessionManager::new()));
    let db = get_diesel_pool().await;

    let rocket = rocket::build()
        .mount("/static", rocket::routes![statics::statics])
        .mount(
            "/",
            rocket::routes![
                statics::favicon,
                async_view,
                season_standings,
                season_brackets,
                season_qualifiers,
                season_redirect,
                season_history,
                home,
                player_detail,
                player_detail_by_id,
                bracket_detail,
                helper_bot
            ],
        )
        .attach(Template::custom(|e| {
            e.tera.register_filter("option_default", option_default);
        }))
        .manage(state)
        .manage(session_manager)
        .manage(oauth_client)
        .manage(db);
    let rocket = api::build_rocket(rocket);
    let rocket = auth::build_rocket(rocket);
    let rocket = internal_api::build_rocket(rocket, bri_sender);

    let ignited = rocket.ignite().await?;
    info!("Rocket config: {:?}", ignited.config());
    let s = ignited.shutdown();
    let jh = tokio::spawn(ignited.launch());

    // if you don't assign this .recv() value to anything, it get dropped immediately
    // i am pretty sure it gets dropped at the end of `recv()`?
    let _x = shutdown.recv().await;
    s.notify();
    if let Err(_) = tokio::time::timeout(Duration::from_secs(5), jh).await {
        info!("Rocket didn't shutdown in a timely manner, dropping anyway");
    } else {
        info!("Rocket shut down promptly");
    }
    Ok(())
}
