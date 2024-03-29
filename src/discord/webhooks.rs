use log::warn;
use nmg_league_bot::config::CONFIG;
use std::sync::Arc;
use twilight_http::client::Client;
use twilight_http::request::channel::webhook::ExecuteWebhook;
use twilight_http::response::marker::EmptyBody;
use twilight_http::Response;
use twilight_model::channel::Webhook;
use twilight_model::id::marker::WebhookMarker;
use twilight_model::id::Id;
use twilight_util::link::webhook::parse;

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
async fn get_webhook_by_url(client: &Arc<Client>, url: String) -> Result<WebhookInfo, String> {
    let (id, tokeno) = parse(&url).map_err(|e| e.to_string())?;
    let token = tokeno.ok_or(format!("No token found for webhook {}", id))?;
    let resp: Response<Webhook> = match client.webhook(id).token(&token).await {
        Ok(r) => r,
        Err(e) => {
            let er = format!("Error fetching webhook {}: {}", id, e);
            warn!("{}", er);
            return Err(er);
        }
    };
    match resp.model().await {
        Ok(w) => Ok(WebhookInfo {
            id: w.id,
            token: w.token.ok_or("Webhook with no token".to_string())?,
        }),
        Err(e) => Err(e.to_string()),
    }
}

impl Webhooks {
    pub async fn new(client: Arc<Client>) -> Result<Self, String> {
        let async_channel = get_webhook_by_url(&client, CONFIG.async_webhook.clone()).await?;
        let error_channel = get_webhook_by_url(&client, CONFIG.error_webhook.clone()).await?;

        Ok(Self {
            http_client: client,
            async_channel,
            error_channel,
        })
    }

    pub async fn execute_webhook(&self, ew: ExecuteWebhook<'_>) -> Result<(), String> {
        let resp: Response<EmptyBody> = ew.await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            Err(format!("Error executing webhook: {:?}", resp.text().await))
        } else {
            Ok(())
        }
    }

    fn _execute_webhook<'a>(&'a self, webhook: &'a WebhookInfo) -> ExecuteWebhook<'a> {
        self.http_client.execute_webhook(webhook.id, &webhook.token)
    }

    pub fn prepare_execute_async(&self) -> ExecuteWebhook {
        self._execute_webhook(&self.async_channel)
    }

    pub async fn message_async(&self, content: &str) -> Result<(), String> {
        self.execute_webhook(
            self.prepare_execute_async()
                .content(content)
                .map_err(|e| e.to_string())?,
        )
        .await
    }

    pub async fn message_error(&self, content: &str) -> Result<(), String> {
        self.execute_webhook(
            self._execute_webhook(&self.error_channel)
                .content(content)
                .map_err(|e| e.to_string())?,
        )
        .await
    }
}
