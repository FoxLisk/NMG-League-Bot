use crate::constants::GUILD_ID_VAR;
use crate::discord::ADMIN_ROLE_NAME;
use crate::Webhooks;
use dashmap::DashMap;
use sqlx::SqlitePool;
use std::env;
use std::sync::Arc;
use twilight_cache_inmemory::InMemoryCache;
use twilight_http::client::InteractionClient;
use twilight_http::Client;
use twilight_model::guild::Role;
use twilight_model::id::marker::{ApplicationMarker, ChannelMarker, GuildMarker, UserMarker};
use twilight_model::id::Id;
use twilight_model::user::User;
use twilight_standby::Standby;

pub(crate) struct DiscordState {
    pub cache: InMemoryCache,
    pub client: Client,
    pub pool: SqlitePool,
    pub webhooks: Webhooks,
    pub standby: Arc<Standby>,
    // this isn't handled by the cache b/c it is not updated via Gateway events
    private_channels: DashMap<Id<UserMarker>, Id<ChannelMarker>>,
    application_id: Id<ApplicationMarker>,
    gid: Id<GuildMarker>,
}

impl DiscordState {
    pub(super) fn new(
        cache: InMemoryCache,
        client: Client,
        aid: Id<ApplicationMarker>,
        pool: SqlitePool,
        webhooks: Webhooks,
        standby: Arc<Standby>,
    ) -> Self {
        let gid_s = env::var(GUILD_ID_VAR).unwrap();
        let gid = Id::<GuildMarker>::new(gid_s.parse::<u64>().unwrap());
        Self {
            cache,
            client,
            pool,
            webhooks,
            standby,
            application_id: aid,
            private_channels: Default::default(),
            gid,
        }
    }

    pub(super) fn interaction_client<'a>(&'a self) -> InteractionClient<'a> {
        self.client.interaction(self.application_id.clone())
    }

    pub(super) async fn get_private_channel(
        &self,
        user: Id<UserMarker>,
    ) -> Result<Id<ChannelMarker>, String> {
        if let Some(id) = self.private_channels.get(&user) {
            return Ok(id.clone());
        }

        let created = self
            .client
            .create_private_channel(user.clone())
            .exec()
            .await
            .map_err(|e| e.to_string())?;
        if created.status().is_success() {
            let chan = created.model().await.map_err(|e| e.to_string())?;
            self.private_channels.insert(user, chan.id.clone());
            Ok(chan.id)
        } else {
            Err(format!(
                "Error result creating private channel: {}",
                created.status()
            ))
        }
    }

    pub(crate) fn get_admin_role(&self, guild_id: Id<GuildMarker>) -> Option<Role> {
        let roles = self.cache.guild_roles(guild_id)?;

        for role_id in roles.value() {
            if let Some(role) = self.cache.role(*role_id) {
                if role.name == ADMIN_ROLE_NAME {
                    // cloning pulls it out of the reference, unlocking the cache
                    return Some(role.resource().clone());
                }
            }
        }
        None
    }

    // making this async now so when i inevitably add an actual HTTP request fallback it's already async
    pub(crate) async fn has_admin_role(
        &self,
        user_id: Id<UserMarker>,
        guild_id: Id<GuildMarker>,
    ) -> Result<bool, String> {
        let role = self.get_admin_role(guild_id).ok_or(format!(
            "Error: Cannot find admin role for guild {}",
            guild_id
        ))?;
        let member = self
            .cache
            .member(guild_id, user_id)
            .ok_or(format!("Error: cannot find member {}", user_id))?
            .value()
            .clone();
        Ok(member.roles().contains(&role.id))
    }

    // convenience for website which i have negative interest in adding guild info to
    pub(crate) async fn has_nmg_league_admin_role(
        &self,
        user_id: Id<UserMarker>,
    ) -> Result<bool, String> {
        self.has_admin_role(user_id, self.gid.clone()).await
    }

    pub(crate) async fn get_user(&self, user_id: Id<UserMarker>) -> Result<Option<User>, String> {
        Ok(self.cache.user(user_id).map(|u| u.value().clone()))
    }
}
