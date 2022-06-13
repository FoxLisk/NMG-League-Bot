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
use rocket::http::{Cookie, CookieJar};
use rocket::request::{FromRequest, Outcome};
use rocket::response::Redirect;
use rocket::{get, Request, State};
use rocket_dyn_templates::Template;
use tokio::sync::{Mutex as AsyncMutex};
use tokio::time::{Duration, Instant};
use tokio_stream::StreamExt;

use crate::constants::{
    AUTHORIZE_URL_VAR, CLIENT_ID_VAR, CLIENT_SECRET_VAR, DISCORD_AUTHORIZE_URL, DISCORD_TOKEN_URL,
};
use crate::db::get_pool;
use crate::models::race_run::RaceRunState;
use crate::shutdown::Shutdown;
use crate::web::session_manager::SessionManager as _SessionManager;
use crate::web::session_manager::SessionToken;
use crate::discord::discord_state::DiscordState;
use serde::Serialize;
use sqlx::SqlitePool;
use std::str::FromStr;
use twilight_model::id::Id;
use twilight_model::id::marker::UserMarker;

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
            .add_scope(Scope::new(
                "identify".to_string()
            ))
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
            let st = SessionToken::new(cookie);
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
            _ =>  Outcome::Forward(())
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
        let suffix = filename.rsplit('.').next().unwrap();
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
    error_description: Option<String>,
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
        Ok(resp) => {
            match resp.model().await {
                Ok(cu) => cu,
                Err(e) => {
                    println!("Error deserializing CurrentUser: {}", e);
                    return Err(redirect);
                }
            }
        }
        Err(e) => {
            println!("Error getting user info: {}", e);
            return Err(redirect);
        }
    };
    let is_admin = role_checker.has_nmg_league_admin_role(user_info.id.clone()).await
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

        cookies.add(Cookie::new(SESSION_COOKIE_NAME, st.into_string()));
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

#[get("/asyncs")]
async fn async_view(
    _a: Admin,
    pool: &State<SqlitePool>,
    discord_state: &State<Arc<DiscordState>>,
) -> Template {
    let mut qr = sqlx::query!(
        r#"SELECT
            race_id,
            rr.uuid as rr_uuid,
            racer_id,
            filenames,
            rr.state as "rr_state: RaceRunState",
            vod,
            r.state as "race_state!"
        FROM race_runs rr
        LEFT JOIN races r
        ON rr.race_id = r.id
        ORDER BY r.created DESC;"#
    )
    .fetch(pool.inner());


    #[derive(Serialize)]
    struct Run {
        run_uuid: String,
        racer_name: String,
        filenames: String,
        run_state: RaceRunState,
        vod: Option<String>,
    }

    #[derive(Serialize)]
    struct Race {
        state: String,
        p1: Run,
        p2: Run
    }

    let mut runs = HashMap::<i64, Vec<Run>>::new();
    let mut race_state = HashMap::<i64, String>::new();

    while let Some(row_res) = qr.next().await {
        let row = match row_res {
            Ok(r) => r,
            Err(e) => {
                println!("Async view: error fetching row: {}", e);
                continue;
            }
        };
        let uid = match Id::<UserMarker>::from_str(&row.racer_id) {
            Ok(u) => u,
            Err(e) => {
                println!("Error parsing racer id {}: {}", row.racer_id, e);
                continue;
            }
        };
        let username = discord_state.get_user(uid).await.ok().flatten().map(|u| u.name).unwrap_or("Unknown".to_string());
        let run = Run {
            run_uuid: row.rr_uuid,
            racer_name: username,
            filenames: row.filenames,
            vod: row.vod,
            run_state: row.rr_state,
        };
        runs
            .entry(row.race_id)
            .or_insert(vec![])
            .push(run);
        race_state.insert(row.race_id, row.race_state);
    }
    let mut races = HashMap::<String, Vec<Race>>::new();
    for (id, mut runs) in runs.into_iter() {
        let state = match race_state.remove(&id) {
            Some(s) => s,
            None => {
                println!("Run found for missing race?");
                continue;
            }
        };
        runs.sort_by(|a, b| a.racer_name.partial_cmp(&b.racer_name).unwrap());
        let p1 = match runs.pop() {
            Some(r) => r,
            None => {
                println!("Not enough runs for race {}", id);
                continue;
            }
        };
        let p2 = match runs.pop() {
            Some(r) => r,
            None => {
                println!("Not enough runs for race {}", id);
                continue;
            }
        };
        races.entry(state.clone()).or_insert(vec![]).push(Race {
            state, p1, p2
        });
    }

    #[derive(Serialize)]
    struct Context {
        races_by_state: HashMap<String, Vec<Race>>,
    }

    Template::render("asyncs", Context { races_by_state: races })
}


pub(crate) async fn launch_website(
    state: Arc<DiscordState>,
    mut shutdown: tokio::sync::broadcast::Receiver<Shutdown>,
) {
    let pool = get_pool().await.unwrap();

    let oauth_client = OauthClient::new();
    let session_manager: Arc<AsyncMutex<SessionManager>> =
        Arc::new(AsyncMutex::new(SessionManager::new()));

    let rocket = rocket::build()
        .mount("/static", rocket::routes![statics])
        .mount(
            "/",
            rocket::routes![
                login_page,
                discord_login,
                index_logged_in,
                index_logged_out,
                async_view
            ],
        )
        .attach(Template::fairing())
        .manage(state)
        .manage(session_manager)
        .manage(oauth_client)
        .manage(pool);

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
            println!("Rocket did shutdown in a timely manner, actually");
        }
    });
}
