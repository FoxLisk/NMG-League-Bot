use crate::constants::{
    ADMIN_WEBHOOK_VAR, ASYNC_WEBHOOK_VAR, TOKEN_VAR,
};
use std::sync::Arc;
use twilight_http::client::Client;
use twilight_http::Response;
use twilight_model::channel::Webhook;
use twilight_model::id::marker::WebhookMarker;
use twilight_model::id::Id;
use regex::Regex;
use serenity::prelude::TypeMapKey;
use twilight_http::request::channel::webhook::ExecuteWebhook;
use std::fmt::Display;
use twilight_http::response::marker::EmptyBody;

#[derive(Clone)]
pub(crate) struct Webhooks {
    http_client: Arc<Client>,
    async_channel: Webhook,
    admin_channel: Webhook,
}

impl TypeMapKey for Webhooks {
    type Value = Self;
}

pub fn parse_webhook(url: &str) -> Option<(u64, String)> {
    // https://discord.com/api/webhooks/983193713075449937/ZH4A70mJHr1e6So1MVc-Ksh7Fott_-miU0i20b3-ibPg7_UAwrLYb3eAVAh3nMQ2-LJB
    let re = Regex::new(r#"https?://discord.com/api/webhooks/(\d+)/([\w_-]+)/?"#).unwrap();
    let groups = re.captures(url)?;
    let id = groups.get(1)?.as_str().parse::<u64>().ok()?;
    let token = groups.get(2)?.as_str().to_string();
    Some((id, token))
}

async fn get_webhook_by_url(client: &Arc<Client>, url: String) -> Result<Webhook, String> {
    let (id, token) = parse_webhook(&url).ok_or("Error parsing webhook url".to_string())?;
    let resp: Response<Webhook> = match client.webhook(Id::new(id)).token(&token).exec().await {
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
        let async_webhook_url = std::env::var(ASYNC_WEBHOOK_VAR)
            .unwrap();
        let admin_webhook_url = std::env::var(ADMIN_WEBHOOK_VAR)
            .unwrap();

        let async_channel = get_webhook_by_url(&client, async_webhook_url).await?;
        let admin_channel = get_webhook_by_url(&client, admin_webhook_url).await?;

        Ok(Self {
            http_client: client,
            async_channel,
            admin_channel,
        })
    }

    async fn execute_webhook(&self, content: &str, ew: ExecuteWebhook<'_>) -> Result<(), String> {
        let resp: Response<EmptyBody> = ew
            .content(content)
            .map_err(|e| e.to_string())?
            .exec()
            .await
            .map_err(|e| e.to_string())?;
        if ! resp.status().is_success() {
            Err(format!("Error executing webhook: {:?}", resp.text().await))
        } else {
            Ok(())
        }
    }

    fn _execute_webhook<'a>(&'a self, webhook: &'a Webhook) -> ExecuteWebhook<'a> {
        self.http_client.execute_webhook(webhook.id, webhook.token.as_ref().unwrap())
    }

    pub(crate) fn execute_async<'a>(&'a self) -> ExecuteWebhook<'a> {
        self._execute_webhook(&self.async_channel)
    }

    pub(crate) fn execute_admin<'a>(&'a self) -> ExecuteWebhook<'a> {
        self._execute_webhook(&self.admin_channel)
    }

    pub(crate) async fn message_async(&self, content: &str) -> Result<(), String> {
        self.execute_webhook(
            content,
            self.execute_async()
        ).await
    }
    //
    //
    // pub(crate) async fn execute_async(&self, content: &str) -> Result<(), String> {
    //     self.execute_webhook(content, &self.async_channel).await
    // }
    //
    // pub(crate) async fn execute_admin(&self, content: &str) -> Result<(), String> {
    //     self.execute_webhook(content, &self.admin_channel).await
    // }

}
