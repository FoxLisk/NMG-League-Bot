use crate::discord::constants::UPDATE_USER_INFO_CMD;
use crate::discord::discord_state::DiscordState;
use crate::shutdown::Shutdown;
use async_trait::async_trait;
use diesel::SqliteConnection;
use log::{debug, error, info, warn};
use nmg_league_bot::config::CONFIG;
use nmg_league_bot::models::bracket_race_infos::BracketRaceInfo;
use nmg_league_bot::models::player::Player;
use nmg_league_bot::models::season::Season;
use nmg_league_bot::utils::racetime_base_url;
use nmg_league_bot::{NMGLeagueBotError, RaceTimeBotError};
use racetime::handler::RaceContext;
use racetime::model::{ChatMessage, RaceData};
use racetime::{authorize_with_host, Bot, Error, HostInfo, RaceHandler, StartRace};
use racetime_api::types::Race;
use regex::Regex;
use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::Arc;
use tokio::sync::broadcast::Receiver as BroadcastReceiver;
use tokio::sync::mpsc::Receiver as MpscReceiver;
use tokio::sync::Mutex;
use tokio::time::Instant;

fn host_info() -> HostInfo {
    HostInfo::new(
        &CONFIG.racetime_host,
        CONFIG.racetime_port,
        CONFIG.racetime_secure,
    )
}

fn url_from_slug(slug: &str) -> String {
    let base_url = racetime_base_url();
    format!("{base_url}/{}/{slug}", CONFIG.racetime_category)
}

fn slug_from_url(url: &str) -> Option<String> {
    let base_url = format!("{}/{}", racetime_base_url(), CONFIG.racetime_category);
    let url_regex: Result<Regex, regex::Error> = Regex::new(&format!(r"{base_url}/([^/]+)"));
    let re = match url_regex {
        Ok(re) => re,
        Err(e) => {
            warn!("Error with url_regex, unable to check existing races for racetime room: {e}");
            return None;
        }
    };
    re.captures(url)
        .map(|c| c.get(1))
        .flatten()
        .map(|m| m.as_str().to_string())
}

pub async fn run_bot(
    state: Arc<DiscordState>,
    races_to_create: MpscReceiver<BracketRaceInfo>,
    mut sd: BroadcastReceiver<Shutdown>,
) {
    println!("Racetime bot starting...");
    let hi = host_info();

    let rt_state = Arc::new(RacetimeState::new(state.clone()));
    {
        let mut db = state.diesel_cxn().await.unwrap();
        if let Some(s) = Season::get_active_season(db.deref_mut()).unwrap() {
            // TODO: configurable window?
            if let Ok(in_flight_races) =
                s.get_unfinished_races_after(chrono::Duration::minutes(30), db.deref_mut())
            {
                for (bri, _) in in_flight_races {
                    if let Some(thing) = &bri.racetime_gg_url {
                        if let Some(slug) = slug_from_url(thing) {
                            rt_state.learn_about_bri(slug, &bri).await.ok();
                        }
                    }
                }
            };
        }
    }
    // I don't love using configuration to determine the category. i suppose we could use
    // the info in the current season, but then if there's no current season we just don't run the bot?
    // that would be fine-ish but we'd have to think about stopping and restarting the bot, etc.
    // just gonna use conf for now
    let b = Bot::new_with_host(
        hi,
        &CONFIG.racetime_category,
        &CONFIG.racetime_client_id,
        &CONFIG.racetime_client_secret,
        rt_state.clone(),
    )
    .await
    .unwrap();

    tokio::spawn(create_rooms(races_to_create, rt_state.clone()));
    drop(state);
    match b.run_until::<Handler, _, _>(sd.recv()).await {
        Ok(shutdown) => {
            info!("RaceTime bot shut down normally");
            // Does it make the intent more obvious to write it like this, or just write it as
            // Ok(_shutdown) and let the scope rules drop it?
            drop(shutdown);
        }
        Err(e) => {
            error!("RaceTime bot failed with error: {e}");
        }
    }
}

struct Token<'a> {
    access_token: Option<String>,
    expires_at: Option<Instant>,
    host_info: &'a HostInfo,
    client: &'a reqwest::Client,
}
impl<'a> Token<'a> {
    fn new(host_info: &'a HostInfo, client: &'a reqwest::Client) -> Self {
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
                self.expires_at = Some((Instant::now() + d) - tokio::time::Duration::from_secs(10));
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

    async fn get_token(&mut self) -> Result<String, RaceTimeBotError> {
        self.maybe_refresh().await?;
        self.access_token
            .as_ref()
            .map(Clone::clone)
            .ok_or(RaceTimeBotError::NoAuthToken)
    }
}

// TODO: is a BRI really the thing I want to ship out here?
async fn create_rooms(mut events: MpscReceiver<BracketRaceInfo>, state: Arc<RacetimeState>) {
    let hi = host_info();

    let client = reqwest::Client::default();
    let mut token = Token::new(&hi, &client);
    while let Some(mut bri) = events.recv().await {
        match handle_one_bri(&mut bri, &mut token, &state.discord_state).await {
            Ok(slug) => {
                if let Err(e) = state.learn_about_bri(slug, &bri).await {
                    warn!("Error telling RacetimeState about new room for BRI: {e}");
                }
            }
            Err(e) => {
                warn!("Error creating race: {e} - while handling {bri:?}");
            }
        }
    }
    info!("create_rooms fell out of loop");
}

async fn handle_one_bri(
    bri: &mut BracketRaceInfo,
    token: &mut Token<'_>,
    state: &Arc<DiscordState>,
) -> Result<String, NMGLeagueBotError> {
    let at = token.get_token().await?;
    let slug = create_room_for_race(&bri, &at, &token.host_info, &token.client, &state).await?;
    let url = url_from_slug(&slug);
    bri.racetime_gg_url = Some(url.clone());
    let mut conn = state.diesel_cxn().await?;
    bri.update(conn.deref_mut())?;
    Ok(slug)
}
/// the returned String is the slug of the race room
async fn create_room_for_race(
    bri: &BracketRaceInfo,
    access_token: &str,
    host_info: &HostInfo,
    client: &reqwest::Client,
    state: &Arc<DiscordState>,
) -> Result<String, NMGLeagueBotError> {
    let mut db = state.diesel_cxn().await?;
    let szn = Season::get_from_bracket_race_info(bri, db.deref_mut())?;
    if szn.rtgg_category_name != CONFIG.racetime_category {
        warn!("Can't create racetime room: category mismatch!");
        return Err(RaceTimeBotError::InvalidCategory)?;
    }
    let sr = StartRace {
        goal: szn.rtgg_goal_name,
        goal_is_custom: false,
        team_race: false,
        invitational: false,
        unlisted: false,
        info_user: "info user text".to_string(),
        info_bot: "info bot text".to_string(),
        require_even_teams: false,
        start_delay: 15,
        time_limit: 3,
        time_limit_auto_complete: false,
        streaming_required: !cfg!(feature = "testing"),
        auto_start: true,
        allow_comments: true,
        hide_comments: false,
        allow_prerace_chat: true,
        allow_midrace_chat: true,
        allow_non_entrant_chat: true,
        chat_message_delay: 0,
    };
    sr.start_with_host(&host_info, access_token, client, &CONFIG.racetime_category)
        .await
        // this double From is dumb and annoying but is it _wrong_?
        .map_err(RaceTimeBotError::from)
        .map_err(From::from)
}

struct RacetimeState {
    discord_state: Arc<DiscordState>,
    // TODO: clear this out, this is technically a memory leak
    // TODO: dashmap?
    race_name_to_bracket_race_info_id: Mutex<HashMap<String, i32>>,
}

impl RacetimeState {
    fn new(discord_state: Arc<DiscordState>) -> Self {
        let race_name_to_bracket_race_info_id = Mutex::new(HashMap::new());
        Self {
            discord_state,
            race_name_to_bracket_race_info_id,
        }
    }
    async fn learn_about_bri(
        &self,
        slug: String,
        bri: &BracketRaceInfo,
    ) -> Result<(), RaceTimeBotError> {
        let mut map = self.race_name_to_bracket_race_info_id.lock().await;
        if let Some(e) = map.get(&slug) {
            if e.clone() != bri.id {
                return Err(RaceTimeBotError::ConflictingSlug(e.clone()));
            }
        }
        debug!(
            "RaceTime bot: on the lookout for BRI {} at slug {slug}",
            bri.id
        );
        map.insert(slug, bri.id);
        Ok(())
    }

    async fn get_bri_id_by_slug(&self, slug: &str) -> Option<i32> {
        let map = self.race_name_to_bracket_race_info_id.lock().await;
        map.get(slug).cloned()
    }
}

struct Handler {
    bri_id: i32,
}

impl Handler {
    fn get_players(
        &self,
        db: &mut SqliteConnection,
    ) -> Result<(Player, Player), NMGLeagueBotError> {
        let race = BracketRaceInfo::get_by_id(self.bri_id, db)?.race(db)?;
        race.players(db).map_err(From::from)
    }

    async fn handle_new_race_room(
        &self,
        ctx: &RaceContext<RacetimeState>,
        rd: &RaceData,
    ) -> Result<(), NMGLeagueBotError> {
        if let Err(e) = ctx
            .send_message(
                "Hello and welcome to your race! Auto-start is on. \
            Admins and ZSR staff can type !promote to get race monitor status if needed. \
            Have fun and good luck!",
            )
            .await
        {
            warn!("{e}");
        }
        // if we can't get a db or figure out who the players are, the room really is an error
        let mut db = ctx.global_state.discord_state.diesel_cxn().await?;
        let (p1, p2) = self.get_players(db.deref_mut())?;
        // if we can't *invite* them, however, it's probably better to just make the room open
        // and let them know about it in discord
        let mut success = true;

        for player in vec![&p1, &p2] {
            match &player.racetime_user_id {
                Some(id) => {
                    if let Err(e) = ctx.invite_user(id).await {
                        success = false;
                    }
                }
                None => {
                    success = false;
                }
            }
        }
        if !success {
            // oh this just fails if the *send* fails, it doesn't wait for a response.
            // see Self::error for error handling such as it is
            if let Err(_e) = ctx.set_open().await {
                // retry once i guess?
                if let Err(e) = ctx.set_open().await {
                    warn!("Error setting racetime room {rd:?} open. Giving up cause idk what to do now. {e}");
                    return Err(RaceTimeBotError::RaceTimeError(e))?;
                }
            }
            info!("Set racetime room {} to open", rd.slug);
        }

        let p1_m = p1.mention_or_name();
        let p2_m = p2.mention_or_name();
        ctx.global_state
            .discord_state
            .client
            .create_message(CONFIG.racetime_room_posting_channel_id)
            .content(&format!(
                "{p1_m} {p2_m} your race room is ready! {}",
                url_from_slug(&rd.slug)
            ))?
            .await?;
        info!("Discord message sent");
        Ok(())
    }

    /// sends a message and logs an error if there is any.
    async fn send_message(&self, content: &str, ctx: &RaceContext<RacetimeState>) {
        if let Err(e) = ctx.send_message(content).await {
            warn!("Error sending message: {e}");
        }
    }

    async fn handle_promote(
        &self,
        ctx: &RaceContext<RacetimeState>,
        cmd_name: String,
        args: Vec<String>,
        is_moderator: bool,
        is_monitor: bool,
        msg: &ChatMessage,
    ) -> Result<(), NMGLeagueBotError> {
        if is_monitor {
            self.send_message("You are already a race monitor.", ctx)
                .await;
        }
        if is_moderator {
            self.send_message(
                "You are a moderator and cannot also be made a race monitor.",
                ctx,
            )
            .await;
        }
        let mut db = ctx.global_state.discord_state.diesel_cxn().await?;
        if let Some(ud) = &msg.user {
            // unfortunately we have reached a terrible moment: we now have Players who are known not to play SadgeBusiness
            // (i.e. ZSR admins)
            let p = Player::get_by_rtgg_id(&ud.id, db.deref_mut())?
                .map(|p| p.discord_id().ok())
                .flatten();
            match p {
                Some(p_id) => {
                    if ctx
                        .global_state
                        .discord_state
                        .has_any_nmg_league_role(
                            p_id,
                            &vec![
                                CONFIG.discord_admin_role_name.as_str(),
                                CONFIG.discord_zsr_role_name.as_str(),
                            ][..],
                        )
                        .await
                    {
                        let rd = ctx.data().await;
                        let is_entrant = rd.entrants.iter().any(|e| e.user.id == ud.id);

                        // apparently you can only add people as monitors if they're entrants in the race
                        if !is_entrant {
                            if let Err(e) = ctx.invite_user(&ud.id).await {
                                warn!(
                                    "Error inviting {} to add them as a race monitor: {e}",
                                    ud.id
                                );
                            }
                        }

                        if let Err(e) = ctx.add_monitor(&ud.id).await {
                            warn!("Error adding {} as race monitor: {e}", ud.id);
                        }
                        // but don't kick someone if they were already an entrant
                        if !is_entrant {
                            if let Err(e) = ctx.remove_entrant(&ud.id).await {
                                warn!("Error removing {} as entrant after adding them as race monitor: {e}", ud.id);
                            }
                        }
                    } else {
                        self.send_message(
                            "Sorry, you don't appear to be allowed to do that. If you think this is an error, \
                                     reach out to FoxLisk on Discord.", ctx).await;
                    }
                }
                None => {
                    self.send_message(
                        &format!(
                            "I'm afraid I don't recognize you. Please set your racetime info by using \
                            /{} in the discord.", UPDATE_USER_INFO_CMD), ctx
                    ).await;
                }
            }
        } else {
            warn!("Got !promote command with no user data...?");
            self.send_message("An unreasonable error occurred, sorry. Try again?", ctx)
                .await;
        }

        Ok(())
    }
}

#[async_trait]
impl RaceHandler<RacetimeState> for Handler {
    async fn should_handle(race_data: &RaceData, state: Arc<RacetimeState>) -> Result<bool, Error> {
        Ok(state.get_bri_id_by_slug(&race_data.slug).await.is_some())
    }

    async fn new(ctx: &RaceContext<RacetimeState>) -> Result<Self, Error> {
        let rd = ctx.data().await;
        let handler = match ctx.global_state.get_bri_id_by_slug(&rd.slug).await {
            Some(id) => Self { bri_id: id },
            None => {
                return Err(Error::Custom(Box::new(RaceTimeBotError::MissingBRI(
                    rd.slug.clone(),
                ))));
            }
        };
        handler
            .handle_new_race_room(ctx, &rd)
            .await
            .map_err(Into::<Error>::into)?;
        debug!("Handle new race room succeeded!");
        Ok(handler)
    }

    async fn command(
        &mut self,
        ctx: &RaceContext<RacetimeState>,
        cmd_name: String,
        args: Vec<String>,
        _is_moderator: bool,
        is_monitor: bool,
        msg: &ChatMessage,
    ) -> Result<(), Error> {
        info!("Got command {cmd_name} with args {args:?}");

        match cmd_name.as_str() {
            "hello" => {
                self.send_message("Hello!", ctx).await;
            }
            #[cfg(feature = "testing")]
            "data" => {
                debug!("{:?}", ctx.data().await);
            }
            "promote" => {
                debug!("promoting");
                if let Err(e) = self
                    .handle_promote(ctx, cmd_name, args, _is_moderator, is_monitor, msg)
                    .await
                {
                    warn!("Error handling promotion request: {e}");
                }
            }
            _ => {
                debug!("Unknown command !{cmd_name}");
            }
        }
        Ok(())
    }

    async fn error(
        &mut self,
        _ctx: &RaceContext<RacetimeState>,
        errors: Vec<String>,
    ) -> Result<(), Error> {
        // maybe there's some kind of error I care about? the issue is that if you don't override
        // this method, the default implementation fails on *any* error from rtgg, including things like
        // "you tried to invite a user with the wrong ID"
        // especially since this is called asynchronously that's very annoying.
        debug!("Error from rtgg: {errors:?}");
        Ok(())
    }
}
