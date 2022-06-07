use crate::models::uuid_string;
use crate::oauth2::TokenResponse as OTR;
use crate::web::TokenResponse;
use aliri_braid::braid;
use serenity::model::id::UserId;
use std::collections::HashMap;
use tokio::time::Instant;

pub(crate) const SESSION_COOKIE_NAME: &str = "session_token";

#[braid]
pub(crate) struct SessionToken;

impl SessionToken {
    fn create() -> Self {
        Self::new(uuid_string())
    }
}

#[derive(Debug)]
pub(crate) enum UserNotAuthenticated {
    SessionTokenNotFound,
    TokenExpired,
    TokenRefreshFailed,
}

pub(crate) struct SessionManager {
    session: HashMap<SessionToken, (UserId, Instant)>,
}

// N.B. maybe using UserTokens isn't ideal here, and we should have some kind of internal type
// it's gonna include a lot of copying, at least - although, again, performance doesn't actually matter, so w/e

impl SessionManager {
    pub(crate) fn new() -> Self {
        Self {
            session: Default::default(),
        }
    }

    pub(crate) fn log_in_user(&mut self, uid: UserId, expire_at: Instant) -> SessionToken {
        let t = SessionToken::create();
        self.session.insert(t.clone(), (uid, expire_at));
        t
    }

    /// Returns the user associated with the given session token, if any
    #[allow(unused)]
    pub(crate) fn get_user(
        &mut self,
        st: &SessionTokenRef,
    ) -> Result<UserId, UserNotAuthenticated> {
        let (uid, exp_at) = self
            .session
            .get(st)
            .ok_or(UserNotAuthenticated::SessionTokenNotFound)?;
        if Instant::now() > *exp_at {
            Err(UserNotAuthenticated::TokenExpired)
        } else {
            Ok(uid.clone())
        }
    }
}
