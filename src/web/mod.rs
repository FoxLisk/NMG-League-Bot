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
use tokio::sync::{Mutex as AsyncMutex, RwLock};
use tokio::time::{Duration, Instant};
use tokio_stream::StreamExt;

use crate::constants::{
    AUTHORIZE_URL_VAR, CLIENT_ID_VAR, CLIENT_SECRET_VAR, DISCORD_AUTHORIZE_URL, DISCORD_TOKEN_URL,
    NMG_LEAGUE_GUILD_ID, TOKEN_VAR,
};
use crate::db::get_pool;
use crate::models::{race::RaceState, race_run::RaceRunState};
use crate::shutdown::Shutdown;
use crate::web::session_manager::SessionManager;
use crate::web::session_manager::SessionToken;
use serde::Serialize;
use serenity::http::Http;
use serenity::model::id::{GuildId, RoleId, UserId};
use serenity::model::user::{CurrentUser, User};
use serenity::CacheAndHttp;
use sqlx::SqlitePool;
use std::str::FromStr;

mod session_manager;

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
                serenity::model::oauth2::OAuth2Scope::Identify.to_string(),
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

        let role_checker = match request.guard::<&State<DiscordStateRepository>>().await {
            Outcome::Success(s) => s,
            _ => {
                return Outcome::Forward(());
            }
        };
        if role_checker.has_nmg_league_admin_role(&uid).await {
            Outcome::Success(Admin {})
        } else {
            Outcome::Forward(())
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
    let mut p = Path::new("http/static/").join(file);
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
    role_checker: &State<DiscordStateRepository>,
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
    let http = Http::new(&*user_token);
    let user_info: CurrentUser = match http.get_current_user().await {
        Ok(ui) => {
            println!("Got user info {:?}", ui);
            ui
        }
        Err(e) => {
            println!("Error getting user info: {}", e);
            return Err(redirect);
        }
    };
    let is_admin = role_checker.has_nmg_league_admin_role(&user_info.id).await;

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
    discord_state: &State<DiscordStateRepository>,
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
    struct Row {
        run_uuid: String,
        racer_name: String,
        filenames: String,
        run_state: RaceRunState,
        vod: Option<String>,
    }

    let mut races = HashMap::<String, Vec<Row>>::new();

    while let Some(row_res) = qr.next().await {
        let row = match row_res {
            Ok(r) => r,
            Err(e) => {
                println!("Async view: error fetching row: {}", e);
                continue;
            }
        };
        let uid = match UserId::from_str(&row.racer_id) {
            Ok(u) => u,
            Err(e) => {
                println!("Error parsing racer id {}: {}", row.racer_id, e);
                continue;
            }
        };
        let username = match discord_state.cache_and_http.cache.user(uid) {
            Some(user) => user.name,
            None => match discord_state.get_user(uid).await {
                Some(user) => user.name,
                None => {
                    println!("Cannot find racer with user id {}", uid);
                    "Unknown".to_string()
                }
            },
        };
        let output_row = Row {
            run_uuid: row.rr_uuid,
            racer_name: username,
            filenames: row.filenames,
            vod: row.vod,
            run_state: row.rr_state,
        };
        races
            .entry(row.race_state)
            .or_insert(vec![])
            .push(output_row);
    }

    #[derive(Serialize)]
    struct Context {
        races: HashMap<String, Vec<Row>>,
    }

    Template::render("asyncs", Context { races })
}

struct DiscordStateRepository {
    cache_and_http: Arc<CacheAndHttp>,
    guild_role_map: Arc<RwLock<HashMap<GuildId, RoleId>>>,
}

impl DiscordStateRepository {
    async fn has_nmg_league_admin_role(&self, uid: &UserId) -> bool {
        let u = match self.cache_and_http.cache.user(uid) {
            Some(user) => user,
            None => match self.cache_and_http.http.get_user(uid.0).await {
                Ok(user) => user,
                Err(e) => {
                    println!("Error fetching user: {}", e);
                    return false;
                }
            },
        };

        let gid = GuildId(NMG_LEAGUE_GUILD_ID.parse().unwrap());
        let role_id = {
            match self.guild_role_map.read().await.get(&gid) {
                Some(rid) => rid.clone(),
                None => {
                    println!("No admin role found for nmg league?");
                    return false;
                }
            }
        };

        match u.has_role(&self.cache_and_http, gid, role_id).await {
            Ok(b) => b,
            Err(e) => {
                println!("Error checking user role: {}", e);
                false
            }
        }
    }

    async fn get_user(&self, uid: UserId) -> Option<User> {
        match self.cache_and_http.cache.user(&uid) {
            Some(u) => Some(u),
            None => self.cache_and_http.http.get_user(uid.0).await.ok(),
        }
    }
}

pub(crate) async fn launch_website(
    cache_and_http: Arc<CacheAndHttp>,
    guild_role_map: Arc<RwLock<HashMap<GuildId, RoleId>>>,
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
        .manage(DiscordStateRepository {
            cache_and_http,
            guild_role_map,
        })
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
