use crate::Webhooks;
use bb8::{Pool, PooledConnection, RunError};
use dashmap::DashMap;
use diesel::ConnectionError;
use log::warn;
use nmg_league_bot::config::CONFIG;
use nmg_league_bot::db::DieselConnectionManager;
use nmg_league_bot::models::player::Player;
use nmg_league_bot::twitch_client::TwitchClientBundle;
use nmg_league_bot::utils::ResultErrToString;
use nmg_league_bot::{ChannelConfig, NMGLeagueBotError};
use racetime_api::client::RacetimeClient;
use std::fmt::Display;
use std::sync::Arc;
use thiserror::Error;
use twilight_cache_inmemory::InMemoryCache;
use twilight_http::client::InteractionClient;
use twilight_http::Client;
use twilight_model::gateway::payload::incoming::InteractionCreate;
use twilight_model::guild::Role;
use twilight_model::http::interaction::InteractionResponse;
use twilight_model::id::marker::{
    ApplicationMarker, ChannelMarker, GuildMarker, InteractionMarker, RoleMarker, UserMarker,
};
use twilight_model::id::Id;
use twilight_model::user::User;
use twilight_standby::Standby;

pub struct DiscordState {
    pub cache: InMemoryCache,
    pub discord_client: Arc<Client>,
    diesel_pool: Pool<DieselConnectionManager>,
    pub webhooks: Webhooks,
    pub standby: Arc<Standby>,
    // this isn't handled by the cache b/c it is not updated via Gateway events
    private_channels: DashMap<Id<UserMarker>, Id<ChannelMarker>>,
    application_id: Id<ApplicationMarker>,
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
        guild_id: Id<GuildMarker>,
    },
}

#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait DiscordOperations {
    async fn submit_error<S: Display + Send + 'static>(&self, error: S);

    fn interaction_client<'a>(&'a self) -> InteractionClient<'a>;

    async fn get_private_channel(&self, user: Id<UserMarker>) -> Result<Id<ChannelMarker>, String>;

    async fn has_admin_role(
        &self,
        user_id: Id<UserMarker>,
        guild_id: Id<GuildMarker>,
    ) -> Result<bool, DiscordStateError>;

    async fn has_nmg_league_admin_role(
        &self,
        user_id: Id<UserMarker>,
    ) -> Result<bool, DiscordStateError>;

    async fn has_nmg_league_role_by_name(
        &self,
        user_id: Id<UserMarker>,
        role_name: &str,
    ) -> Result<bool, DiscordStateError>;

    async fn has_any_nmg_league_role<'a>(
        &self,
        user_id: Id<UserMarker>,
        role_names: &[&'a str],
    ) -> bool;
    /// convenience method for getting a user's info from the twilight cache
    fn get_user(&self, user_id: Id<UserMarker>) -> Option<User>;

    async fn best_name_for(&self, user_id: Id<UserMarker>) -> String;

    async fn create_response(
        &self,
        interaction_id: Id<InteractionMarker>,
        token: &str,
        resp: &InteractionResponse,
    ) -> Result<(), twilight_http::Error>;

    async fn application_command_run_by_admin(
        &self,
        ac: &Box<InteractionCreate>,
    ) -> Result<bool, String>;

    async fn create_response_err_to_str(
        &self,
        interaction_id: Id<InteractionMarker>,
        token: &str,
        resp: &InteractionResponse,
    ) -> Result<(), String>;

    // this method should be removed when i manage diesel connections per-event better
    async fn diesel_cxn<'a>(
        &'a self,
    ) -> Result<PooledConnection<'a, DieselConnectionManager>, RunError<ConnectionError>>;

    async fn get_player_pfp(&self, p: &Player) -> Result<Option<String>, NMGLeagueBotError>;
}

impl DiscordState {
    pub fn new(
        cache: InMemoryCache,
        client: Arc<Client>,
        aid: Id<ApplicationMarker>,
        diesel_pool: Pool<DieselConnectionManager>,
        webhooks: Webhooks,
        standby: Arc<Standby>,
        racetime_client: RacetimeClient,
        twitch_client_bundle: TwitchClientBundle,
    ) -> Self {
        let channel_config = ChannelConfig::new_from_env();

        Self {
            cache,
            discord_client: client,
            diesel_pool,
            webhooks,
            standby,
            application_id: aid,
            private_channels: Default::default(),
            channel_config,
            racetime_client,
            twitch_client_bundle,
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

    fn has_role(
        &self,
        user_id: Id<UserMarker>,
        guild_id: Id<GuildMarker>,
        role_id: Id<RoleMarker>,
    ) -> Result<bool, DiscordStateError> {
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
        let role =
            self.get_role_by_name(guild_id, role_name)
                .ok_or(DiscordStateError::RoleNotFound {
                    role_name: role_name.to_string(),
                    guild_id,
                })?;
        self.has_role(user_id, guild_id, role.id)
    }

    /// Returns Some(URL) if the user has a pfp, otherwise None or an error
    async fn get_player_discord_pfp(
        &self,
        p: &Player,
    ) -> Result<Option<String>, NMGLeagueBotError> {
        let disc_id = p.discord_id()?;
        let user_info = self.discord_client.user(disc_id).await?;
        let user = user_info.model().await?;
        Ok(user.avatar.map(|i| {
            let ext = if i.is_animated() { ".gif" } else { ".png" };
            format!("https://cdn.discordapp.com/avatars/{disc_id}/{i}{ext}?size=128")
        }))
    }
}

#[async_trait::async_trait]
impl DiscordOperations for DiscordState {
    async fn submit_error<S: Display + Send>(&self, error: S) {
        let msg = format!("{error}");
        if let Err(e) = self.webhooks.message_error(&msg).await {
            warn!("Error sending message {msg} to error channel: {e}");
        }
    }

    fn interaction_client(&self) -> InteractionClient<'_> {
        self.discord_client.interaction(self.application_id.clone())
    }

    async fn get_private_channel(&self, user: Id<UserMarker>) -> Result<Id<ChannelMarker>, String> {
        if let Some(id) = self.private_channels.get(&user) {
            return Ok(id.clone());
        }

        let created = self
            .discord_client
            .create_private_channel(user.clone())
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

    // making this async now so when i inevitably add an actual HTTP request fallback it's already async
    async fn has_admin_role(
        &self,
        user_id: Id<UserMarker>,
        guild_id: Id<GuildMarker>,
    ) -> Result<bool, DiscordStateError> {
        self.has_role_by_name(user_id, guild_id, &CONFIG.discord_admin_role_name)
    }

    // convenience for website which i have negative interest in adding guild info to
    async fn has_nmg_league_admin_role(
        &self,
        user_id: Id<UserMarker>,
    ) -> Result<bool, DiscordStateError> {
        self.has_admin_role(user_id, CONFIG.guild_id.clone()).await
    }

    async fn has_nmg_league_role_by_name(
        &self,
        user_id: Id<UserMarker>,
        role_name: &str,
    ) -> Result<bool, DiscordStateError> {
        self.has_role_by_name(user_id, CONFIG.guild_id.clone(), role_name)
    }

    async fn has_any_nmg_league_role<'a>(
        &self,
        user_id: Id<UserMarker>,
        role_names: &[&'a str],
    ) -> bool {
        for role_name in role_names {
            if let Ok(true) = self.has_nmg_league_role_by_name(user_id, role_name).await {
                return true;
            }
        }
        false
    }

    fn get_user(&self, user_id: Id<UserMarker>) -> Option<User> {
        self.cache.user(user_id).map(|u| u.value().clone())
    }

    /// gets best available name for this user (without hitting the discord http api)
    ///
    /// order of preference:
    ///
    /// 1. the name the player set for themself
    /// 2. the player's nickname in the NMG League server
    /// 3. the player's global nickname on discord
    /// 4. the player's discord username
    async fn best_name_for(&self, user_id: Id<UserMarker>) -> String {
        // maybe would be good to like care about errors in here?
        if let Ok(mut conn) = self.diesel_cxn().await {
            if let Ok(Some(p)) = Player::get_by_discord_id(&user_id.to_string(), &mut conn) {
                return p.name;
            }
        }
        if let Some(member) = self.cache.member(CONFIG.guild_id, user_id) {
            if let Some(nick) = member.nick() {
                return nick.to_string();
            }
        }
        if let Some(user) = self.get_user(user_id) {
            if let Some(global) = user.global_name {
                return global;
            }
            return user.name;
        }
        "unknown".to_string()
    }

    async fn create_response(
        &self,
        interaction_id: Id<InteractionMarker>,
        token: &str,
        resp: &InteractionResponse,
    ) -> Result<(), twilight_http::Error> {
        self.interaction_client()
            .create_response(interaction_id, token, resp)
            .await
            .map(|_| ())
    }

    async fn application_command_run_by_admin(
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
    async fn create_response_err_to_str(
        &self,
        interaction_id: Id<InteractionMarker>,
        token: &str,
        resp: &InteractionResponse,
    ) -> Result<(), String> {
        self.create_response(interaction_id, token, resp)
            .await
            .map_err(|e| e.to_string())
    }

    async fn diesel_cxn<'a>(
        &'a self,
    ) -> Result<PooledConnection<DieselConnectionManager>, RunError<ConnectionError>> {
        // return Err(RunError::User(ConnectionError::BadConnection("asdf".to_string())));
        let pc = self.diesel_pool.get().await;
        pc
    }

    /// gets the player's PFP. returns an URL to the image if found, None if player doesnt have a pfp,
    /// error if there's an error
    ///
    /// in principle this might someday check discord and also twitch but right now it only checks discord
    async fn get_player_pfp(&self, p: &Player) -> Result<Option<String>, NMGLeagueBotError> {
        self.get_player_discord_pfp(p).await
    }
}
