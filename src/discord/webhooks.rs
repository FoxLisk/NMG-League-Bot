use crate::constants::{ADMIN_WEBHOOK_VAR, ASYNC_WEBHOOK_VAR, TOKEN_VAR};
use std::sync::Arc;
use twilight_http::client::Client;
use twilight_http::request::channel::webhook::ExecuteWebhook;
use twilight_http::response::marker::EmptyBody;
use twilight_http::Response;
use twilight_model::channel::Webhook;
use twilight_util::link::webhook::parse;

#[derive(Clone)]
pub(crate) struct Webhooks {
    http_client: Arc<Client>,
    async_channel: Webhook,
    admin_channel: Webhook,
}

async fn get_webhook_by_url(client: &Arc<Client>, url: String) -> Result<Webhook, String> {
    let (id, tokeno) = parse(&url).map_err(|e| e.to_string())?;
    let token = match tokeno {
        Some(t) => t,
        None => {
            return Err(format!("No token found for webhook {}", id));
        }
    };
    let resp: Response<Webhook> = match client.webhook(id).token(&token).exec().await {
        Ok(r) => r,
        Err(e) => {
            let er = format!("Error fetching webhook {}: {}", id, e);
            println!("{}", er);
            return Err(er);
        }
    };
    resp.model().await.map_err(|e| e.to_string())
}

impl Webhooks {
    pub(crate) async fn new() -> Result<Self, String> {
        let client = Arc::new(Client::new(std::env::var(TOKEN_VAR).unwrap()));
        let async_webhook_url = std::env::var(ASYNC_WEBHOOK_VAR).unwrap();
        let admin_webhook_url = std::env::var(ADMIN_WEBHOOK_VAR).unwrap();

        let async_channel = get_webhook_by_url(&client, async_webhook_url).await?;
        let admin_channel = get_webhook_by_url(&client, admin_webhook_url).await?;

        Ok(Self {
            http_client: client,
            async_channel,
            admin_channel,
        })
    }

    async fn execute_webhook_with_content(
        &self,
        content: &str,
        ew: ExecuteWebhook<'_>,
    ) -> Result<(), String> {
        let resp: Response<EmptyBody> = ew
            .content(content)
            .map_err(|e| e.to_string())?
            .exec()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            Err(format!("Error executing webhook: {:?}", resp.text().await))
        } else {
            Ok(())
        }
    }

    pub(crate) async fn execute_webhook(&self, ew: ExecuteWebhook<'_>) -> Result<(), String> {
        let resp: Response<EmptyBody> = ew.exec().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            Err(format!("Error executing webhook: {:?}", resp.text().await))
        } else {
            Ok(())
        }
    }

    fn _execute_webhook<'a>(&'a self, webhook: &'a Webhook) -> ExecuteWebhook<'a> {
        self.http_client
            .execute_webhook(webhook.id, webhook.token.as_ref().unwrap())
    }

    pub(crate) fn prepare_execute_async<'a>(&'a self) -> ExecuteWebhook<'a> {
        self._execute_webhook(&self.async_channel)
    }

    pub(crate) fn prepare_execute_admin<'a>(&'a self) -> ExecuteWebhook<'a> {
        self._execute_webhook(&self.admin_channel)
    }

    pub(crate) async fn message_async(&self, content: &str) -> Result<(), String> {
        self.execute_webhook(
            self.prepare_execute_async()
                .content(content)
                .map_err(|e| e.to_string())?,
        )
        .await
    }
}
