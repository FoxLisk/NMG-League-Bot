use nmg_league_bot::config::CONFIG;
use nmg_league_bot::RaceTimeBotError;
use racetime::{authorize_with_host, HostInfo};
use std::time::Duration;
use tokio::time::Instant;

pub struct Token<'a> {
    access_token: Option<String>,
    expires_at: Option<Instant>,
    host_info: &'a HostInfo,
    client: &'a reqwest::Client,
}

impl<'a> Token<'a> {
    pub fn new(host_info: &'a HostInfo, client: &'a reqwest::Client) -> Self {
        Self {
            access_token: None,
            expires_at: None,
            host_info,
            client,
        }
    }
    async fn update_token(&mut self) -> Result<(), RaceTimeBotError> {
        match authorize_with_host(
            self.host_info,
            &CONFIG.racetime_client_id,
            &CONFIG.racetime_client_secret,
            self.client,
        )
        .await
        {
            Ok((t, d)) => {
                self.access_token = Some(t);
                // pretend it expires a little early, to be safe
                self.expires_at = Some((Instant::now() + d) - Duration::from_secs(10));
                Ok(())
            }
            Err(e) => {
                // assume any error means we don't have a valid token anymore, either
                self.access_token = None;
                self.expires_at = None;
                Err(From::from(e))
            }
        }
    }

    async fn maybe_refresh(&mut self) -> Result<(), RaceTimeBotError> {
        if let Some(ea) = &self.expires_at {
            if ea > &Instant::now() {
                // if we have a token and it hasn't expired, no-op
                return Ok(());
            }
        }
        // no token or expired token fall through to refresh
        self.update_token().await
    }

    pub async fn get_token(&mut self) -> Result<String, RaceTimeBotError> {
        self.maybe_refresh().await?;
        self.access_token
            .as_ref()
            .map(Clone::clone)
            .ok_or(RaceTimeBotError::NoAuthToken)
    }

    pub fn client(&self) -> &'a reqwest::Client {
        &self.client
    }

    pub fn host_info(&self) -> &'a HostInfo {
        &self.host_info
    }
}
