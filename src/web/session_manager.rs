use aliri_braid::braid;
use nmg_league_bot::utils::uuid_string;
use std::collections::HashMap;
use tokio::time::Instant;

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
}

pub struct SessionManager<T> {
    session: HashMap<SessionToken, (T, Instant)>,
}

// N.B. maybe using UserTokens isn't ideal here, and we should have some kind of internal type
// it's gonna include a lot of copying, at least - although, again, performance doesn't actually matter, so w/e

impl<T: Clone> SessionManager<T> {
    pub(crate) fn new() -> Self {
        Self {
            session: Default::default(),
        }
    }

    pub(crate) fn log_in_user(&mut self, uid: T, expire_at: Instant) -> SessionToken {
        let t = SessionToken::create();
        self.session.insert(t.clone(), (uid, expire_at));
        t
    }

    /// Returns the user associated with the given session token, if any
    pub(crate) fn get_user(&mut self, st: &SessionTokenRef) -> Result<T, UserNotAuthenticated> {
        let (uid, exp_at) = self
            .session
            .get(st)
            .ok_or(UserNotAuthenticated::SessionTokenNotFound)?;
        if Instant::now() > *exp_at {
            Err(UserNotAuthenticated::TokenExpired)
        } else {
            let t: T = uid.clone();
            Ok(t)
        }
    }
}
