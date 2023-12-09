use crate::discord::discord_state::DiscordState;
use crate::web::session_manager::SessionToken;
use crate::web::{BaseContext, ConnectionWrapper, SessionManager, SESSION_COOKIE_NAME};
use log::{debug, info, warn};
use nmg_league_bot::config::CONFIG;
use nmg_league_bot::constants::{DISCORD_AUTHORIZE_URL, DISCORD_TOKEN_URL};
use oauth2::basic::{BasicClient, BasicErrorResponseType, BasicTokenType};
use oauth2::reqwest::async_http_client;
use oauth2::url::Url;
use oauth2::{
    AuthUrl, AuthorizationCode, Client, ClientId, ClientSecret, CsrfToken, EmptyExtraTokenFields,
    RedirectUrl, RequestTokenError, RevocationErrorResponseType, Scope, StandardErrorResponse,
    StandardRevocableToken, StandardTokenIntrospectionResponse, StandardTokenResponse,
    TokenResponse as OauthTokenResponse, TokenUrl,
};
use rocket::http::{Cookie, CookieJar, Status};
use rocket::request::{FromRequest, Outcome};
use rocket::response::Redirect;
use rocket::time::Duration;
use rocket::{get, Build, Rocket};
use rocket::{Request, State};
use rocket_dyn_templates::{context, Template};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::Instant;

type TokenResponse = StandardTokenResponse<EmptyExtraTokenFields, BasicTokenType>;

type _OauthClient = Client<
    StandardErrorResponse<BasicErrorResponseType>,
    TokenResponse,
    BasicTokenType,
    StandardTokenIntrospectionResponse<EmptyExtraTokenFields, BasicTokenType>,
    StandardRevocableToken,
    StandardErrorResponse<RevocationErrorResponseType>,
>;

pub struct OauthClient {
    client: _OauthClient,
    states: Arc<Mutex<HashMap<String, Instant>>>,
}

impl OauthClient {
    const STATE_TIMEOUT_SECS: u64 = 60 * 5;
    fn _is_expired(created: Instant) -> bool {
        let delta = Instant::now() - created;
        delta.as_secs() > Self::STATE_TIMEOUT_SECS
    }

    pub fn new() -> Self {
        Self {
            client: BasicClient::new(
                ClientId::new(CONFIG.discord_client_id.clone()),
                Some(ClientSecret::new(CONFIG.discord_client_secret.clone())),
                AuthUrl::new(DISCORD_AUTHORIZE_URL.to_string()).unwrap(),
                Some(TokenUrl::new(DISCORD_TOKEN_URL.to_string()).unwrap()),
            )
            .set_redirect_uri(RedirectUrl::new(CONFIG.discord_authorize_url.clone()).unwrap()),
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
                warn!("States mutex poisoned: {}", e);
            }
        }
        auth_url
    }

    /// Takes a discord oauth "code" and "state" and tries to turn them into a token
    /// Tries to verify the state but in case of mutex poisoning it's possible to validate a code
    /// with an unexpected state
    async fn exchange_code(&self, code: String, state: String) -> Option<TokenResponse> {
        match self.states.lock() {
            Ok(mut states) => {
                if let Some(created) = states.remove(&state) {
                    if Self::_is_expired(created) {
                        debug!("Token expired");
                        return None;
                    }
                } else {
                    debug!("Unexpected state: {}", state);
                    return None;
                }
            }
            Err(e) => {
                warn!("States were poisoned: failing open. Error: {}", e);
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
                        warn!("Server response error: {e}");
                    }
                    _ => {
                        warn!("Other exchange error: {e}");
                    }
                }
                None
            }
        }
    }
}

pub(super) struct Admin {}

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
                debug!("No session cookie");
                return Outcome::Forward(());
            }
        };
        let st = SessionToken::new(cookie.to_string());
        let sm_lock = match request
            .guard::<&State<Arc<tokio::sync::Mutex<SessionManager>>>>()
            .await
        {
            Outcome::Success(s) => s,
            _ => {
                return Outcome::Forward(());
            }
        };
        let uid = {
            let mut sm = sm_lock.lock().await;
            match sm.get_user(&st) {
                Ok(u) => u,
                Err(e) => {
                    info!("User not found for session token {}: {:?}", st, e);
                    request.cookies().remove(Cookie::named(SESSION_COOKIE_NAME));
                    return Outcome::Forward(());
                }
            }
        };

        let state = match request.guard::<&State<Arc<DiscordState>>>().await {
            Outcome::Success(s) => s,
            _ => {
                return Outcome::Forward(());
            }
        };
        match state.has_nmg_league_admin_role(uid).await {
            Ok(true) => Outcome::Success(Admin {}),
            Ok(false) => {
                // N.B. would it be better to just get this lock once?
                let mut sm = sm_lock.lock().await;
                let un = state
                    .get_user(uid)
                    .map(|u| u.global_name.unwrap_or(u.name))
                    .unwrap_or("Unknown username".to_string());
                debug!("Logging out {un} because they no longer have admin status");
                sm.log_out_user(&st);
                request.cookies().remove(Cookie::named(SESSION_COOKIE_NAME));
                Outcome::Failure((Status::Forbidden, ()))
            }
            _ => Outcome::Forward(()),
        }
    }
}

// this has a "bug" where it doesn't check if you're logged in or not. this should probably
// have a redirect if you have an Admin guard. but i don't feel like thinking about it right now.
#[get("/login")]
async fn login_page(client: &State<OauthClient>, mut db: ConnectionWrapper<'_>) -> Template {
    let url = client.auth_url();
    let bc = BaseContext::new(&mut db, &None);
    let ctx = context! {
        url: url.to_string(),
        base_context: bc,
    };
    Template::render("login", ctx)
}

#[get("/discord_login?<code>&<state>&<error_description>")]
async fn discord_login(
    code: Option<String>,
    state: String,
    #[allow(unused_variables)] error_description: Option<String>,
    client: &State<OauthClient>,
    session_manager: &State<Arc<tokio::sync::Mutex<SessionManager>>>,
    role_checker: &State<Arc<DiscordState>>,
    cookies: &CookieJar<'_>,
) -> Result<Template, Redirect> {
    let redirect = Redirect::to(rocket::uri!("/", login_page));
    let got_code = code.ok_or(Redirect::to(rocket::uri!("/", login_page)))?;
    let res: TokenResponse = match client.exchange_code(got_code, state).await {
        Some(tr) => tr,
        None => {
            info!("Failed to exchange code");
            return Err(redirect);
        }
    };
    let user_token = match res.token_type() {
        BasicTokenType::Bearer => {
            format!("Bearer {}", res.access_token().secret())
        }
        _ => {
            warn!("Unexpected token type {:?}", res.token_type());
            return Err(redirect);
        }
    };

    let client = twilight_http::client::Client::new(user_token);

    let user_info = match client.current_user().await {
        Ok(resp) => match resp.model().await {
            Ok(cu) => cu,
            Err(e) => {
                warn!("Error deserializing CurrentUser: {}", e);
                return Err(redirect);
            }
        },
        Err(e) => {
            warn!("Error getting user info: {}", e);
            return Err(redirect);
        }
    };
    let is_admin = role_checker
        .has_nmg_league_admin_role(user_info.id.clone())
        .await
        .map_err(|e| {
            warn!("Error checking for admin status: {}", e);
            Redirect::to(rocket::uri!("/", login_page))
        })?;

    if is_admin {
        let st = {
            let mut sm = session_manager.lock().await;
            sm.log_in_user(
                user_info.id,
                Instant::now()
                    + res
                        .expires_in()
                        .unwrap_or(tokio::time::Duration::from_secs(60 * 60)),
            )
        };
        let cookie = Cookie::build(SESSION_COOKIE_NAME, st.to_string())
            .max_age(Duration::days(30))
            .finish();
        cookies.add(cookie);
        info!("User {} has logged in as an admin", user_info.name);
        Ok(Template::render(
            "login_redirect",
            HashMap::<String, String>::new(),
        ))
    } else {
        info!("Non-admin user");
        Err(redirect)
    }
}

pub fn build_rocket(rocket: Rocket<Build>) -> Rocket<Build> {
    rocket.mount("/", rocket::routes![login_page, discord_login])
}
