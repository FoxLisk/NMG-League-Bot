use std::sync::Arc;

use nmg_league_bot::config::CONFIG;
use nmg_league_bot::NMGLeagueBotError;
use twilight_http::request::scheduled_event::{
    CreateGuildScheduledEvent, UpdateGuildScheduledEvent,
};
use twilight_http::Client;
use twilight_model::guild::scheduled_event::{GuildScheduledEvent, PrivacyLevel};
use twilight_model::id::marker::{GuildMarker, ScheduledEventMarker};
use twilight_model::id::Id;

#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait EventManager {
    async fn get_guild_scheduled_events(
        &self,
        guild_id: Id<GuildMarker>,
    ) -> Result<Vec<GuildScheduledEvent>, NMGLeagueBotError>;

    fn update_scheduled_event(
        &self,
        guild_id: Id<GuildMarker>,
        event_id: Id<twilight_model::id::marker::ScheduledEventMarker>,
    ) -> twilight_http::request::scheduled_event::UpdateGuildScheduledEvent<'_>;
    fn create_scheduled_event(&self, guild_id: Id<GuildMarker>) -> CreateGuildScheduledEvent<'_>;
}

// i have to give this a different name than EventManager but that's the only reasonable name so
//
// idk man
pub struct EventClient {
    client: Arc<Client>,
}

impl EventClient {
    pub fn new() -> Self {
        let c = Client::new(CONFIG.helper_bot_discord_token.clone());

        Self {
            client: Arc::new(c),
        }
    }
}

#[async_trait::async_trait]
impl EventManager for EventClient {
    async fn get_guild_scheduled_events(
        &self,
        guild_id: Id<GuildMarker>,
    ) -> Result<Vec<GuildScheduledEvent>, NMGLeagueBotError> {
        let req = self.client.guild_scheduled_events(guild_id);
        let resp = req.await?;

        let data = resp.models().await?;
        Ok(data)
    }

    fn update_scheduled_event(
        &self,
        guild_id: Id<GuildMarker>,
        event_id: Id<ScheduledEventMarker>,
    ) -> UpdateGuildScheduledEvent<'_> {
        self.client.update_guild_scheduled_event(guild_id, event_id)
    }

    fn create_scheduled_event(&self, guild_id: Id<GuildMarker>) -> CreateGuildScheduledEvent<'_> {
        self.client
            .create_guild_scheduled_event(guild_id, PrivacyLevel::GuildOnly)
    }
}
