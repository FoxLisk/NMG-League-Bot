use itertools::Itertools;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use oauth2::basic::{BasicClient, BasicErrorResponseType, BasicTokenType};
use oauth2::reqwest::async_http_client;
use oauth2::url::Url;
use oauth2::{
    AuthUrl, AuthorizationCode, Client, ClientId, ClientSecret, CsrfToken, EmptyExtraTokenFields,
    RedirectUrl, RequestTokenError, RevocationErrorResponseType, Scope, StandardErrorResponse,
    StandardRevocableToken, StandardTokenIntrospectionResponse, StandardTokenResponse,
    TokenResponse as OauthTokenResponse, TokenUrl,
};
use rocket::fs::NamedFile;
use rocket::http::{Cookie, CookieJar, Status};
use rocket::request::{FromRequest, Outcome};
use rocket::response::Redirect;
use rocket::{get, Request, State};
use rocket_dyn_templates::Template;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::{Duration, Instant};

use crate::constants::{
    AUTHORIZE_URL_VAR, CLIENT_ID_VAR, CLIENT_SECRET_VAR, DISCORD_AUTHORIZE_URL, DISCORD_TOKEN_URL,
};
use crate::db::{get_diesel_pool, DieselConnectionManager};
use crate::discord::discord_state::DiscordState;
use nmg_league_bot::models::bracket_races::BracketRace;
use nmg_league_bot::models::bracket_rounds::BracketRound;
use nmg_league_bot::models::brackets::Bracket;
use nmg_league_bot::models::player::Player;
use nmg_league_bot::models::race::{Race, RaceState};
use nmg_league_bot::models::race_run::{RaceRun, RaceRunState};
use nmg_league_bot::models::season::Season;
use crate::schema;
use crate::shutdown::Shutdown;
use crate::web::session_manager::SessionManager as _SessionManager;
use crate::web::session_manager::SessionToken;
use bb8::Pool;
use diesel::prelude::*;
use rocket_dyn_templates::tera::{to_value, try_get_value, Value};
use serde::Serialize;
use std::ops::DerefMut;
use twilight_model::id::marker::UserMarker;
use twilight_model::id::Id;
use nmg_league_bot::models::bracket_race_infos::BracketRaceInfo;

mod session_manager;

type SessionManager = _SessionManager<Id<UserMarker>>;

const SESSION_COOKIE_NAME: &str = "nmg_league_session";

#[macro_export]
macro_rules! uri {
    ($($t:tt)*) => (rocket::uri!("/", crate::web:: $($t)*))
}

type TokenResponse = StandardTokenResponse<EmptyExtraTokenFields, BasicTokenType>;

type _OauthClient = Client<
    StandardErrorResponse<BasicErrorResponseType>,
    TokenResponse,
    BasicTokenType,
    StandardTokenIntrospectionResponse<EmptyExtraTokenFields, BasicTokenType>,
    StandardRevocableToken,
    StandardErrorResponse<RevocationErrorResponseType>,
>;

struct OauthClient {
    client: _OauthClient,
    states: Arc<Mutex<HashMap<String, Instant>>>,
}

impl OauthClient {
    const STATE_TIMEOUT_SECS: u64 = 60 * 5;
    fn _is_expired(created: Instant) -> bool {
        let delta = Instant::now() - created;
        delta.as_secs() > Self::STATE_TIMEOUT_SECS
    }

    fn new() -> Self {
        Self {
            client: BasicClient::new(
                ClientId::new(std::env::var(CLIENT_ID_VAR).unwrap()),
                Some(ClientSecret::new(std::env::var(CLIENT_SECRET_VAR).unwrap())),
                AuthUrl::new(DISCORD_AUTHORIZE_URL.to_string()).unwrap(),
                Some(TokenUrl::new(DISCORD_TOKEN_URL.to_string()).unwrap()),
            )
            .set_redirect_uri(RedirectUrl::new(std::env::var(AUTHORIZE_URL_VAR).unwrap()).unwrap()),
            states: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn auth_url(&self) -> Url {
        let (auth_url, csrf_token) = self
            .client
            .authorize_url(CsrfToken::new_random)
            // Set the desired scopes.
            .add_scope(Scope::new("identify".to_string()))
            .url();
        let thing = self.states.lock();
        match thing {
            Ok(mut states) => {
                states.insert(csrf_token.secret().clone(), Instant::now());
            }
            Err(e) => {
                println!("States mutex poisoned: {}", e);
            }
        }
        auth_url
    }

    async fn exchange_code(&self, code: String, state: String) -> Option<TokenResponse> {
        match self.states.lock() {
            Ok(mut states) => {
                if let Some(created) = states.remove(&state) {
                    if Self::_is_expired(created) {
                        println!("Token expired");
                        return None;
                    }
                } else {
                    println!("Unexpected state: {}", state);
                    return None;
                }
            }
            Err(e) => {
                println!("States were poisoned: failing open. Error: {}", e);
            }
        }

        match self
            .client
            .exchange_code(AuthorizationCode::new(code))
            // Set the PKCE code verifier.
            .request_async(async_http_client)
            .await
        {
            Ok(r) => Some(r),
            Err(e) => {
                match e {
                    RequestTokenError::ServerResponse(e) => {
                        println!("Server response error: {}", e);
                    }
                    _ => {
                        println!("Other exchange error: {}", e);
                    }
                }
                None
            }
        }
    }
}

const STATIC_SUFFIXES: [&str; 8] = [
    &"js", &"css", &"mp3", &"html", &"jpg", &"ttf", &"otf", &"gif",
];

struct Admin {}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Admin {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        if cfg!(feature = "no_auth_website") {
            return Outcome::Success(Admin {});
        }

        let cookie = match request.cookies().get(SESSION_COOKIE_NAME) {
            Some(c) => c.value(),
            None => {
                println!("No session cookie");
                return Outcome::Forward(());
            }
        };
        let sm_lock = match request
            .guard::<&State<Arc<AsyncMutex<SessionManager>>>>()
            .await
        {
            Outcome::Success(s) => s,
            _ => {
                return Outcome::Forward(());
            }
        };
        let uid = {
            let mut sm = sm_lock.lock().await;
            let st = SessionToken::new(cookie.to_string());
            match sm.get_user(&st) {
                Ok(u) => u,
                Err(e) => {
                    println!("User not found for session token {}: {:?}", st, e);
                    return Outcome::Forward(());
                }
            }
        };

        let role_checker = match request.guard::<&State<Arc<DiscordState>>>().await {
            Outcome::Success(s) => s,
            _ => {
                return Outcome::Forward(());
            }
        };
        match role_checker.has_nmg_league_admin_role(uid).await {
            Ok(true) => Outcome::Success(Admin {}),
            _ => Outcome::Forward(()),
        }
    }
}

// Copied from botlisk - not sure the best way to handle reusing this
struct StaticAsset {}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for StaticAsset {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let path = request.uri().path();
        let filename = match path.segments().last() {
            Some(f) => f,
            None => return Outcome::Failure((rocket::http::Status::NotFound, ())),
        };
        let suffix = match filename.rsplit('.').next() {
            None => {
                return Outcome::Failure((rocket::http::Status::NotFound, ()));
            }
            Some(s) => s,
        };
        if STATIC_SUFFIXES.contains(&suffix) {
            Outcome::Success(StaticAsset {})
        } else {
            Outcome::Failure((rocket::http::Status::NotFound, ()))
        }
    }
}

#[get("/<file..>")]
async fn statics(file: PathBuf, _asset: StaticAsset) -> Option<NamedFile> {
    let p = Path::new("http/static/").join(file);
    if !p.exists() {
        println!("{:?} does not exist", p);
        return None;
    }
    NamedFile::open(p).await.ok()
}

#[get("/", rank = 1)]
async fn index_logged_in(_a: Admin) -> Redirect {
    Redirect::to(uri!(async_view))
}

#[get("/<_..>", rank = 2)]
async fn index_logged_out() -> Redirect {
    Redirect::to(uri!(login_page))
}

#[get("/login")]
async fn login_page(client: &State<OauthClient>) -> Template {
    let url = client.auth_url();
    let mut ctx: HashMap<String, String> = Default::default();
    ctx.insert("url".to_string(), url.to_string());
    Template::render("login", ctx)
}

#[get("/discord_login?<code>&<state>&<error_description>")]
async fn discord_login(
    code: Option<String>,
    state: String,
    #[allow(unused_variables)] error_description: Option<String>,
    client: &State<OauthClient>,
    session_manager: &State<Arc<AsyncMutex<SessionManager>>>,
    role_checker: &State<Arc<DiscordState>>,
    cookies: &CookieJar<'_>,
) -> Result<Template, Redirect> {
    println!("code: {:?}", code);
    let redirect = Redirect::to(uri!(login_page));
    let got_code = code.ok_or(Redirect::to(uri!(login_page)))?;
    let res: TokenResponse = match client.exchange_code(got_code, state).await {
        Some(tr) => tr,
        None => {
            println!("Failed to exchange code");
            return Err(redirect);
        }
    };
    let user_token = match res.token_type() {
        BasicTokenType::Bearer => {
            format!("Bearer {}", res.access_token().secret())
        }
        _ => {
            println!("Unexpected token type {:?}", res.token_type());
            return Err(redirect);
        }
    };

    let client = twilight_http::client::Client::new(user_token);

    let user_info = match client.current_user().exec().await {
        Ok(resp) => match resp.model().await {
            Ok(cu) => cu,
            Err(e) => {
                println!("Error deserializing CurrentUser: {}", e);
                return Err(redirect);
            }
        },
        Err(e) => {
            println!("Error getting user info: {}", e);
            return Err(redirect);
        }
    };
    let is_admin = role_checker
        .has_nmg_league_admin_role(user_info.id.clone())
        .await
        .map_err(|e| {
            println!("Error checking for admin status: {}", e);
            Redirect::to(uri!(login_page))
        })?;

    if is_admin {
        let st = {
            let mut sm = session_manager.lock().await;
            sm.log_in_user(
                user_info.id,
                Instant::now() + res.expires_in().unwrap_or(Duration::from_secs(60 * 60)),
            )
        };

        cookies.add(Cookie::new(SESSION_COOKIE_NAME, st.to_string()));
        println!("User {} has logged in as an admin", user_info.name);
        Ok(Template::render(
            "login_redirect",
            HashMap::<String, String>::new(),
        ))
    } else {
        println!("Non-admin user");
        Err(redirect)
    }
}

const DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

// N.B. this should either not be called DiscordState, or we should manage a separate
// connection pool for the website
#[get("/asyncs")]
async fn async_view(_a: Admin, discord_state: &State<Arc<DiscordState>>) -> Template {
    let mut cxn = match discord_state.diesel_cxn().await {
        Ok(c) => c,
        Err(e) => {
            return Template::render("asyncs", Context::error(e.to_string()));
        }
    };
    let query = schema::races::table.inner_join(schema::race_runs::table);
    let results: Vec<(Race, RaceRun)> = match query.load::<(Race, RaceRun)>(cxn.deref_mut()) {
        Ok(r) => r,
        Err(e) => {
            return Template::render("asyncs", Context::error(e.to_string()));
        }
    };

    #[derive(Serialize, Default)]
    struct Context {
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
        p1: Option<ViewRaceRun>,
        p2: Option<ViewRaceRun>,
    }

    #[derive(Serialize)]
    struct ViewRace {
        id: i32,
        state: RaceState,
        p1: ViewRaceRun,
        p2: ViewRaceRun,
    }

    impl ViewRaceBuilder {
        fn from_race(r: Race) -> Self {
            Self {
                id: r.id,
                state: r.state,
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
                    p1,
                    p2,
                })
            } else {
                Ok(ViewRace {
                    id: self.id,
                    state: self.state,
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
                .await
                .ok()
                .flatten()
                .map(|u| u.name)
                .unwrap_or("Unknown".to_string()),
            Err(e) => {
                println!("Error parsing racer id {}", e);
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
    name_and_status: String,
    winner: bool,
    loser: bool,
}

#[derive(Serialize)]
struct DisplayRace {
    player_1: DisplayPlayer,
    player_2: DisplayPlayer,
    scheduled: Option<String>,
}

impl DisplayRace {
    fn new(p1: &Player, p2: &Player, race: &BracketRace, race_info: &BracketRaceInfo) -> Self {
        let outcome = race.outcome().unwrap_or(None);
        use nmg_league_bot::models::bracket_races::Outcome::{P1Win, P2Win};

        let player_1 = match race.player_1_result() {
            Some(r) => DisplayPlayer {
                name_and_status: format!("{} ({})", p1.name.clone(), r),
                winner: outcome == Some(P1Win),
                loser: outcome == Some(P2Win),
            },
            None => DisplayPlayer {
                name_and_status: p1.name.clone(),
                winner: false,
                loser: false,
            },
        };
        let player_2 = match race.player_2_result() {
            Some(r) => DisplayPlayer {
                name_and_status: format!("{} ({})", p2.name.clone(), r),
                winner: outcome == Some(P2Win),
                loser: outcome == Some(P1Win),
            },
            None => DisplayPlayer {
                name_and_status: p2.name.clone(),
                winner: false,
                loser: false,
            },
        };
        let scheduled = match outcome {
            Some(_) => None,
            None => {
                if let Some(utc_dt) = race_info.scheduled() {
                    Some(
                        utc_dt
                            .with_timezone(&chrono_tz::US::Eastern)
                            .format("%A, %B %d at %r (%Z)")
                            .to_string(),
                    )
                } else {
                    None
                }
            }
        };
        Self {
            player_1,
            player_2,
            scheduled,
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
}

#[derive(Serialize)]
struct BracketsContext {
    season: Option<Season>,
    brackets: Vec<DisplayBracket>,
}

fn get_brackets_context(
    conn: &mut SqliteConnection,
) -> Result<BracketsContext, diesel::result::Error> {
    let mut ctx = BracketsContext {
        season: None,
        brackets: vec![],
    };

    let szn = match Season::get_active_season(conn)? {
        Some(szn) => szn,
        None => {
            return Ok(ctx);
        }
    };

    let brackets = szn.brackets(conn)?;

    for bracket in brackets {
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
                    println!("Missing round with id {}", race.round_id);
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

        let disp_b = DisplayBracket { bracket, rounds };
        ctx.brackets.push(disp_b);
    }

    ctx.season = Some(szn);

    Ok(ctx)
}

#[get("/brackets")]
async fn brackets(db: &State<Pool<DieselConnectionManager>>) -> Result<Template, Status> {
    // rocket::http::Status::InternalServerError
    let mut conn = db.get().await.map_err(|_| Status::InternalServerError)?;

    let ctx = get_brackets_context(&mut conn).map_err(|_| Status::InternalServerError)?;

    Ok(Template::render("brackets", ctx))
}

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
    mut shutdown: tokio::sync::broadcast::Receiver<Shutdown>,
) {
    let oauth_client = OauthClient::new();
    let session_manager: Arc<AsyncMutex<SessionManager>> =
        Arc::new(AsyncMutex::new(SessionManager::new()));
    let db = get_diesel_pool().await;

    let rocket = rocket::build()
        .mount("/static", rocket::routes![statics])
        .mount(
            "/",
            rocket::routes![
                login_page,
                discord_login,
                index_logged_in,
                async_view,
                brackets,
                // it's important to keep this at the end, it functions like a 404 catcher
                // TODO that's actually a bug
                index_logged_out,
            ],
        )
        .attach(Template::custom(|e| {
            e.tera.register_filter("option_default", option_default);
        }))
        .manage(state)
        .manage(session_manager)
        .manage(oauth_client)
        .manage(db);

    let ignited = rocket.ignite().await.unwrap();
    println!("Rocket config: {:?}", ignited.config());
    let s = ignited.shutdown();
    let jh = tokio::spawn(ignited.launch());

    tokio::spawn(async move {
        // if you don't assign this .recv() value to anything, it get dropped immediately
        // i am pretty sure it gets dropped at the end of `recv()`?
        let _x = shutdown.recv().await;
        println!("Sending rocket notify");
        s.notify();
        if let Err(_) = tokio::time::timeout(Duration::from_secs(5), jh).await {
            println!("Rocket didn't shutdown in a timely manner, dropping anyway");
        } else {
            println!("Rocket shut down promptly");
        }
    });
}
