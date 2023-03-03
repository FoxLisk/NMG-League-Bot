use crate::discord::constants::{ADMIN_ROLE_NAME, COMMENTARY_ROLE_NAME};
use crate::Webhooks;
use bb8::{Pool, RunError};
use dashmap::DashMap;
use diesel::ConnectionError;
use nmg_league_bot::constants::GUILD_ID_VAR;
use nmg_league_bot::db::DieselConnectionManager;
use nmg_league_bot::twitch_client::TwitchClientBundle;
use nmg_league_bot::utils::{env_var, ResultErrToString};
use nmg_league_bot::ChannelConfig;
use racetime_api::client::RacetimeClient;
use std::ops::DerefMut;
use std::sync::Arc;
use twilight_cache_inmemory::InMemoryCache;
use twilight_http::client::InteractionClient;
use twilight_http::Client;
use twilight_model::gateway::payload::incoming::InteractionCreate;
use twilight_model::guild::Role;
use twilight_model::http::interaction::InteractionResponse;
use twilight_model::id::marker::{ApplicationMarker, ChannelMarker, GuildMarker, InteractionMarker, RoleMarker, UserMarker};
use twilight_model::id::Id;
use twilight_model::user::User;
use twilight_standby::Standby;
use thiserror::Error;

pub struct DiscordState {
    pub cache: InMemoryCache,
    pub client: Client,
    diesel_pool: Pool<DieselConnectionManager>,
    pub webhooks: Webhooks,
    pub standby: Arc<Standby>,
    // this isn't handled by the cache b/c it is not updated via Gateway events
    private_channels: DashMap<Id<UserMarker>, Id<ChannelMarker>>,
    application_id: Id<ApplicationMarker>,
    // this is really embarrassing and at some point i'll regret writing all this code that assumes
    // this is a one-discord bot, maybe
    gid: Id<GuildMarker>,
    pub channel_config: ChannelConfig,
    pub racetime_client: RacetimeClient,
    pub twitch_client_bundle: TwitchClientBundle,
}

#[derive(Error, Debug)]
pub enum DiscordStateError {
    #[error("Member {0} not found")]
    MemberNotFound(Id<UserMarker>),
    #[error("Role {role_name} not found in guild {guild_id}")]
    RoleNotFound {
        role_name: String,
        guild_id: Id<GuildMarker>
    },
}

impl DiscordState {
    pub fn new(
        cache: InMemoryCache,
        client: Client,
        aid: Id<ApplicationMarker>,
        diesel_pool: Pool<DieselConnectionManager>,
        webhooks: Webhooks,
        standby: Arc<Standby>,
        racetime_client: RacetimeClient,
        twitch_client_bundle: TwitchClientBundle,
    ) -> Self {
        let gid_s = env_var(GUILD_ID_VAR);
        let gid = Id::<GuildMarker>::new(gid_s.parse::<u64>().unwrap());
        let channel_config = ChannelConfig::new_from_env();

        Self {
            cache,
            client,
            diesel_pool,
            webhooks,
            standby,
            application_id: aid,
            private_channels: Default::default(),
            gid,
            channel_config,
            racetime_client,
            twitch_client_bundle,
        }
    }

    pub fn interaction_client(&self) -> InteractionClient {
        self.client.interaction(self.application_id.clone())
    }

    pub async fn get_private_channel(
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

    fn get_role_by_name(&self, guild_id: Id<GuildMarker>, role_name: &str) -> Option<Role> {
        let roles = self.cache.guild_roles(guild_id)?;

        for role_id in roles.value() {
            if let Some(role) = self.cache.role(*role_id) {
                if role.name == role_name {
                    // cloning pulls it out of the reference, unlocking the cache
                    return Some(role.resource().clone());
                }
            }
        }
        None
    }

    fn has_role(&self, user_id: Id<UserMarker>, guild_id: Id<GuildMarker>, role_id: Id<RoleMarker>) -> Result<bool, DiscordStateError> {
        let member = self
            .cache
            .member(guild_id, user_id)
            .ok_or(DiscordStateError::MemberNotFound(user_id))?
            .value()
            .clone();
        Ok(member.roles().contains(&role_id))
    }

    fn has_role_by_name(
        &self,
        user_id: Id<UserMarker>,
        guild_id: Id<GuildMarker>,
        role_name: &str,
    ) -> Result<bool, DiscordStateError> {
        let role = self.get_role_by_name(guild_id, role_name).ok_or(DiscordStateError::RoleNotFound {
            role_name: role_name.to_string(),
            guild_id,
        })?;
        self.has_role(user_id, guild_id, role.id)
    }

    // making this async now so when i inevitably add an actual HTTP request fallback it's already async
    pub async fn has_admin_role(
        &self,
        user_id: Id<UserMarker>,
        guild_id: Id<GuildMarker>,
    ) -> Result<bool, DiscordStateError> {
        self.has_role_by_name(user_id, guild_id, ADMIN_ROLE_NAME)
    }

    pub async fn has_commentary_role(
        &self,
        user_id: Id<UserMarker>,
        guild_id: Id<GuildMarker>,
    ) -> Result<bool, DiscordStateError> {
        self.has_role_by_name(user_id, guild_id, COMMENTARY_ROLE_NAME)
    }

    // convenience for website which i have negative interest in adding guild info to
    pub async fn has_nmg_league_admin_role(&self, user_id: Id<UserMarker>) -> Result<bool, DiscordStateError> {
        self.has_admin_role(user_id, self.gid.clone()).await
    }

    pub async fn get_user(&self, user_id: Id<UserMarker>) -> Result<Option<User>, String> {
        Ok(self.cache.user(user_id).map(|u| u.value().clone()))
    }

    pub async fn create_response(
        &self,
        interaction_id: Id<InteractionMarker>,
        token: &str,
        resp: &InteractionResponse,
    ) -> Result<(), twilight_http::Error> {
        self.interaction_client()
            .create_response(interaction_id, token, resp)
            .exec()
            .await
            .map(|_| ())
    }

    pub async fn application_command_run_by_admin(
        &self,
        ac: &Box<InteractionCreate>,
    ) -> Result<bool, String> {
        let gid = ac
            .guild_id
            .ok_or("Create race called outside of guild context".to_string())?;
        let uid = ac
            .author_id()
            .ok_or("Create race called by no one ????".to_string())?;

        self.has_admin_role(uid, gid).await.map_err_to_string()
    }

    /// creates a response and maps any errors to String
    pub async fn create_response_err_to_str(
        &self,
        interaction_id: Id<InteractionMarker>,
        token: &str,
        resp: &InteractionResponse,
    ) -> Result<(), String> {
        self.create_response(interaction_id, token, resp)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn diesel_cxn<'a>(
        &'a self,
    ) -> Result<impl DerefMut<Target = diesel::SqliteConnection> + 'a, RunError<ConnectionError>>
    {
        // return Err(RunError::User(ConnectionError::BadConnection("asdf".to_string())));
        let pc = self.diesel_pool.get().await;
        pc
    }

    pub fn guild_id(&self) -> Id<GuildMarker> {
        self.gid.clone()
    }
}
