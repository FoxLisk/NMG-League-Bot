use log::warn;
use nmg_league_bot::config::CONFIG;
use std::sync::Arc;
use twilight_http::client::Client;
use twilight_http::request::channel::webhook::ExecuteWebhook;
use twilight_http::response::marker::EmptyBody;
use twilight_http::response::DeserializeBodyError;
use twilight_http::Response;
use twilight_model::channel::Webhook;
use twilight_model::id::marker::WebhookMarker;
use twilight_model::id::Id;
use twilight_util::link::webhook::{parse, WebhookParseError};

#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error("invalid webhook URL: {0}")]
    InvalidUrl(#[from] WebhookParseError),
    #[error("webhook {id} URL does not contain a token")]
    MissingToken { id: Id<WebhookMarker> },
    #[error("Discord HTTP request failed: {0}")]
    Http(#[from] twilight_http::Error),
    #[error("could not deserialize a Discord webhook response: {0}")]
    DeserializeResponse(#[from] DeserializeBodyError),
    #[error("Discord rejected webhook execution with HTTP {status}: {body}")]
    ExecutionRejected { status: u16, body: String },
}

#[derive(Clone)]
pub struct Webhooks {
    http_client: Arc<Client>,
    async_channel: WebhookInfo,
    error_channel: WebhookInfo,
}

#[derive(Clone)]
// this structure is because we *really* need webhooks with tokens here, to be able to execute them,
// but the API returns a nullable token, which the twilight API faithfully reproduces, and
// I want zero .unwrap() calls in steady state code
pub struct WebhookInfo {
    pub id: Id<WebhookMarker>,
    pub token: String,
}

// TODO we're up to enough API requests here that we should maybe stop remotely validating every
// new webhook?
async fn get_webhook_by_url(
    client: &Arc<Client>,
    url: String,
) -> Result<WebhookInfo, WebhookError> {
    let (id, tokeno) = parse(&url)?;
    let token = tokeno.ok_or(WebhookError::MissingToken { id })?;
    let resp: Response<Webhook> = match client.webhook(id).token(&token).await {
        Ok(r) => r,
        Err(source) => {
            warn!("Error fetching webhook {id}: {source}");
            return Err(source.into());
        }
    };
    match resp.model().await {
        Ok(w) => Ok(WebhookInfo {
            id: w.id,
            token: w.token.ok_or(WebhookError::MissingToken { id: w.id })?,
        }),
        Err(e) => Err(e.into()),
    }
}

impl Webhooks {
    pub async fn new(client: Arc<Client>) -> Result<Self, WebhookError> {
        let async_channel = get_webhook_by_url(&client, CONFIG.async_webhook.clone()).await?;
        let error_channel = get_webhook_by_url(&client, CONFIG.error_webhook.clone()).await?;

        Ok(Self {
            http_client: client,
            async_channel,
            error_channel,
        })
    }

    pub async fn execute_webhook(&self, ew: ExecuteWebhook<'_>) -> Result<(), WebhookError> {
        let resp: Response<EmptyBody> = ew.await?;
        if !resp.status().is_success() {
            let status = resp.status().get();
            let body = resp.text().await?;
            Err(WebhookError::ExecutionRejected { status, body })
        } else {
            Ok(())
        }
    }

    fn _execute_webhook<'a>(&'a self, webhook: &'a WebhookInfo) -> ExecuteWebhook<'a> {
        self.http_client.execute_webhook(webhook.id, &webhook.token)
    }

    pub fn prepare_execute_async(&self) -> ExecuteWebhook<'_> {
        self._execute_webhook(&self.async_channel)
    }

    pub async fn message_async(&self, content: &str) -> Result<(), WebhookError> {
        self.execute_webhook(self.prepare_execute_async().content(content))
            .await
    }

    pub async fn message_error(&self, content: &str) -> Result<(), WebhookError> {
        self.execute_webhook(self._execute_webhook(&self.error_channel).content(content))
            .await
    }
}
