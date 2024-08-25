use std::{collections::HashMap, sync::Arc};

use bb8::Pool;
use log::{info, warn};
use nmg_league_bot::{
    config::CONFIG, db::DieselConnectionManager, models::bracket_races::BracketRace,
    NMGLeagueBotError,
};
use tokio::sync::broadcast::Receiver;
use tokio_stream::StreamExt;
use twilight_cache_inmemory::InMemoryCache;
use twilight_gateway::{
    stream::{self, ShardEventStream},
    Config, Event, Intents,
};
use twilight_http::{
    request::scheduled_event::{CreateGuildScheduledEvent, UpdateGuildScheduledEvent},
    Client,
};
use twilight_model::{
    guild::scheduled_event::{GuildScheduledEvent, PrivacyLevel},
    id::{
        marker::{GuildMarker, ScheduledEventMarker},
        Id,
    },
};

use crate::shutdown::Shutdown;

pub(super) struct GuildEventConfig {
    pub(super) guild_id: Id<GuildMarker>,
    #[allow(unused)]
    // very useful in debugging but not necessarily logged outside of development, so allow unused
    pub(super) guild_name: String,
}

impl GuildEventConfig {
    /// is this guild interested in tracking this race?
    pub(super) fn should_sync_race(&self, _race: &BracketRace) -> bool {
        true
    }
}

pub(super) struct HelperBot {
    cache: InMemoryCache,
    client: Client,
    diesel_pool: Pool<DieselConnectionManager>,
}

impl HelperBot {
    pub(super) fn new(diesel_pool: Pool<DieselConnectionManager>) -> Self {
        let cache = InMemoryCache::new();
        let client = Client::new(CONFIG.helper_bot_discord_token.clone());
        Self {
            cache,
            client,
            diesel_pool,
        }
    }

    pub(super) async fn get_guild_scheduled_events(
        &self,
        guild_id: Id<GuildMarker>,
    ) -> Result<Vec<GuildScheduledEvent>, NMGLeagueBotError> {
        let req = self.client.guild_scheduled_events(guild_id);
        let resp = req.await?;

        let data = resp.models().await?;
        Ok(data)
    }

    pub(super) fn update_scheduled_event(
        &self,
        guild_id: Id<GuildMarker>,
        event_id: Id<ScheduledEventMarker>,
    ) -> UpdateGuildScheduledEvent<'_> {
        self.client.update_guild_scheduled_event(guild_id, event_id)
    }

    pub(super) fn create_scheduled_event(
        &self,
        guild_id: Id<GuildMarker>,
    ) -> CreateGuildScheduledEvent<'_> {
        self.client
            .create_guild_scheduled_event(guild_id, PrivacyLevel::GuildOnly)
    }

    pub(super) async fn run(bot: Arc<Self>, mut shutdown: Receiver<Shutdown>) {
        // i *think* all I care about out are what guilds I'm in
        let intents = Intents::GUILDS;

        let cfg = Config::builder(CONFIG.helper_bot_discord_token.clone(), intents).build();

        let mut shards = stream::create_recommended(&bot.client, cfg, |_, builder| builder.build())
            .await
            // TODO: surface this unwrap? idk
            .unwrap()
            .collect::<Vec<_>>();

        // N.B. collecting these into a vec and then using `.iter_mut()` is stupid, but idk how to
        // convince the compiler that i have an iterator of mutable references in a simpler way
        let mut events = ShardEventStream::new(shards.iter_mut());

        loop {
            tokio::select! {
                Some((_shard_id, evt)) = events.next() => {
                    match evt {
                        Ok(event) => {
                            bot.cache.update(&event);
                            bot.handle_event(event).await;
                        }
                        Err(e) => {
                            warn!("Got error receiving discord event: {e}");
                            if e.is_fatal() {
                                info!("Helper bot shutting down due to fatal error");
                                break;
                            }
                        }
                    }
                },

                _sd = shutdown.recv() => {
                    info!("Helper bot shutting down...");
                    break;
                }
            }
        }
        info!("Helper bot done");
    }

    /// gets a [GuildEventConfig] for each guild we're currently in
    ///
    /// the CONFIG.guild_id guild will be first
    pub(super) fn guild_event_configs(&self) -> Vec<GuildEventConfig> {
        let mut guild_ids = self
            .cache
            .iter()
            .guilds()
            .map(|guild| (guild.key().clone(), guild.value().name().to_string()))
            .collect::<HashMap<_, _>>();
        let mut out = Vec::with_capacity(guild_ids.len());

        if let Some((guild_id, guild_name)) = guild_ids.remove_entry(&CONFIG.guild_id) {
            out.push(GuildEventConfig {
                guild_id,
                guild_name,
            });
        }
        out.extend(
            guild_ids
                .into_iter()
                .map(|(guild_id, guild_name)| GuildEventConfig {
                    guild_id,
                    guild_name,
                }),
        );
        out
    }

    async fn handle_event(&self, event: Event) {
        match event {
            Event::GuildCreate(gc) => {
                info!("Joined guild {}: {}", gc.id, gc.name);
            }
            _ => {}
        }
    }
}
