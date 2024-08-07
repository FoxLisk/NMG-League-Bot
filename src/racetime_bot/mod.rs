mod token;
use crate::discord::constants::UPDATE_USER_INFO_CMD;
use crate::discord::discord_state::DiscordOperations;
use crate::discord::discord_state::DiscordState;
use crate::shutdown::Shutdown;
use async_trait::async_trait;
use diesel::SqliteConnection;
use log::{debug, error, info, warn};
use nmg_league_bot::config::CONFIG;
use nmg_league_bot::models::asyncs::race_run::Filenames;
use nmg_league_bot::models::bracket_race_infos::{BracketRaceInfo, BracketRaceInfoId};
use nmg_league_bot::models::player::Player;
use nmg_league_bot::models::season::Season;
use nmg_league_bot::utils::racetime_base_url;
use nmg_league_bot::{NMGLeagueBotError, RaceTimeBotError};
use racetime::handler::RaceContext;
use racetime::model::{ChatMessage, RaceData, RaceStatusValue};
use racetime::{Bot, Error, HostInfo, RaceHandler, StartRace};
use regex::Regex;
use serde_json::json;
use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::Arc;
use std::time::Duration;
use token::Token;
use tokio::sync::broadcast::Receiver as BroadcastReceiver;
use tokio::sync::mpsc::{channel, Receiver as MpscReceiver, Receiver, Sender};
use tokio::sync::Mutex;

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
    // TODO: this regex should be cached in a OnceCell
    let url_regex: Result<Regex, regex::Error> = Regex::new(&format!(r"{base_url}/([^/]+)"));
    let re = match url_regex {
        Ok(re) => re,
        Err(e) => {
            warn!("Error with url_regex, unable to check existing races for racetime room: {e}");
            return None;
        }
    };
    re.captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

pub async fn run_bot(
    state: Arc<DiscordState>,
    races_to_create: MpscReceiver<BracketRaceInfoId>,
    mut sd: BroadcastReceiver<Shutdown>,
) {
    info!("Racetime bot starting...");
    let hi = host_info();

    let rt_state = Arc::new(RacetimeState::new(state.clone()));
    {
        let mut db = state.diesel_cxn().await.unwrap();
        if let Some(s) = Season::get_active_season(db.deref_mut()).unwrap() {
            if let Ok(in_flight_races) = s.get_unfinished_races(db.deref_mut()) {
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

async fn create_rooms(mut events: MpscReceiver<BracketRaceInfoId>, state: Arc<RacetimeState>) {
    let hi = host_info();

    let client = reqwest::Client::default();
    let mut token = Token::new(&hi, &client);
    while let Some(bri_id) = events.recv().await {
        let mut bri = {
            let mut db = match state.discord_state.diesel_cxn().await {
                Ok(d) => d,
                Err(e) => {
                    warn!("Error getting db to hydrate BRI: {e}");
                    continue;
                }
            };
            match BracketRaceInfo::get_by_id(bri_id.0, db.deref_mut()) {
                Ok(bri) => bri,
                Err(e) => {
                    warn!("Error getting BRI by id: {e})");
                    continue;
                }
            }
        };
        if bri.racetime_gg_url.is_some() {
            info!("Skipping {bri:?}");
            continue;
        }
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
    let slug = create_room_for_race(&bri, &at, token.host_info(), &token.client(), &state).await?;
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
    let (p1, p2) = bri.race(db.deref_mut())?.players(db.deref_mut())?;
    // TODO: something nicer? bracket name? round number?
    let race_name = format!("NMG League race: {} vs {}", p1.name, p2.name);
    let sr = StartRace {
        goal: szn.rtgg_goal_name,
        goal_is_custom: false,
        team_race: false,
        // invitational gets overridden later if there's an error inviting players
        invitational: true,
        unlisted: false,
        info_user: "".to_string(),
        info_bot: race_name,
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

    /// stop thinking about this slug. returns true if the slug was known
    /// (it would be weird and surprising if this ever returned false!)
    async fn forget_about_bri(&self, slug: &String) -> bool {
        let mut lock = self.race_name_to_bracket_race_info_id.lock().await;
        let existed = lock.remove(slug);
        debug!("RaceTime bot: forgetting about slug {slug}: was {existed:?}");
        existed.is_some()
    }

    async fn get_bri_id_by_slug(&self, slug: &str) -> Option<i32> {
        let map = self.race_name_to_bracket_race_info_id.lock().await;
        map.get(slug).cloned()
    }
}

#[derive(Debug, Clone)]
struct Command {
    cmd_name: String,
    #[allow(unused)]
    args: Vec<String>,
    is_moderator: bool,
    is_monitor: bool,
    msg: ChatMessage,
}

struct HandlerReceiver {
    bri_id: i32,
    should_end: Arc<Mutex<bool>>,
    gethistory_rx: Receiver<Vec<ChatMessage>>,
    command_rx: Receiver<Command>,
}

impl HandlerReceiver {
    async fn set_end(&mut self) {
        let mut lock = self.should_end.lock().await;
        *lock = true;
    }
}

struct Handler {
    should_end: Arc<Mutex<bool>>,
    gethistory_tx: Sender<Vec<ChatMessage>>,
    slug: String,
    command_tx: Sender<Command>,
}

impl Handler {
    fn new(bri_id: i32, slug: String) -> (Self, HandlerReceiver) {
        let (gethistory_tx, gethistory_rx) = channel(10);
        let (command_tx, command_rx) = channel(10);
        let should_end = Arc::new(Mutex::new(false));
        (
            Self {
                gethistory_tx,
                command_tx,
                slug,
                should_end: should_end.clone(),
            },
            HandlerReceiver {
                bri_id,
                command_rx,
                should_end: should_end.clone(),
                gethistory_rx,
            },
        )
    }
    async fn set_end(&mut self) {
        let mut lock = self.should_end.lock().await;
        *lock = true;
    }
}
async fn gethistory(
    handler: &mut HandlerReceiver,
    ctx: &RaceContext<RacetimeState>,
) -> Option<Vec<ChatMessage>> {
    while let Ok(hst) = handler.gethistory_rx.try_recv() {
        debug!("There was a a history message already in the channel {hst:?}");
    }
    if let Err(e) = ctx.send_raw(&json!({"action": "gethistory"})).await {
        warn!("Error sending gethistory: {e}");
        return None;
    }
    tokio::time::timeout(Duration::from_secs(5), handler.gethistory_rx.recv())
        .await
        .ok()
        .flatten()
}

fn get_players(
    handler: &HandlerReceiver,
    db: &mut SqliteConnection,
) -> Result<(Player, Player), NMGLeagueBotError> {
    let race = BracketRaceInfo::get_by_id(handler.bri_id, db)?.race(db)?;
    race.players(db).map_err(From::from)
}

/// called when a race room is created. sends a welcome message, invites players, opens the room
/// if invites fail.  
/// Sends a discord message as well.
async fn initial_setup(
    handler: &HandlerReceiver,
    ctx: &RaceContext<RacetimeState>,
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
    let (p1, p2) = get_players(handler, db.deref_mut())?;

    // if we can't *invite* them, however, it's probably better to just make the room open
    // and let them know about it in discord
    let mut success = true;

    for player in [&p1, &p2] {
        match &player.racetime_user_id {
            Some(id) => {
                if let Err(e) = ctx.invite_user(id).await {
                    warn!("Error inviting user to race: {e}");
                    success = false;
                }
            }
            None => {
                success = false;
            }
        }
    }
    let rd = ctx.data().await;

    if !success {
        // this fails if the *send* fails, it doesn't wait for a response.
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
        .discord_client
        .create_message(CONFIG.racetime_room_posting_channel_id)
        .content(&format!(
            "{p1_m} {p2_m} your race room is ready! {}",
            url_from_slug(&rd.slug)
        ))?
        .await?;

    for player in [&p1, &p2] {
        let filenames = Filenames::new_random();
        if let Err(e) = ctx
            .send_message(&format!(
                "{}: please use the filenames {filenames}",
                player.name
            ))
            .await
        {
            warn!("{e}");
        }
    }
    Ok(())
}

async fn send_message(msg: &str, ctx: &RaceContext<RacetimeState>) {
    if let Err(e) = ctx.send_message(msg).await {
        warn!("Error sending message to racetime room: {e}");
    }
}

async fn handle_promote(
    ctx: &RaceContext<RacetimeState>,
    cmd: Command,
) -> Result<(), NMGLeagueBotError> {
    if cmd.is_monitor {
        send_message("You are already a race monitor.", ctx).await;
    }
    if cmd.is_moderator {
        send_message(
            "You are a moderator and cannot also be made a race monitor.",
            ctx,
        )
        .await;
    }
    let mut db = ctx.global_state.discord_state.diesel_cxn().await?;
    if let Some(ud) = &cmd.msg.user {
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
                    send_message(
                        "Sorry, you don't appear to be allowed to do that. If you think this is an error, \
                                     reach out to FoxLisk on Discord.", ctx).await;
                }
            }
            None => {
                send_message(
                    &format!(
                        "I'm afraid I don't recognize you. Please set your racetime info by using \
                            /{} in the discord.",
                        UPDATE_USER_INFO_CMD
                    ),
                    ctx,
                )
                .await;
            }
        }
    } else {
        warn!("Got !promote command with no user data...?");
        send_message("An unreasonable error occurred, sorry. Try again?", ctx).await;
    }

    Ok(())
}

async fn handle_command(cmd: Command, ctx: &RaceContext<RacetimeState>) {
    match cmd.cmd_name.as_str() {
        "hello" => {
            send_message("Hello!", ctx).await;
        }
        #[cfg(feature = "testing")]
        "data" => {
            debug!("{:?}", ctx.data().await);
        }
        "promote" => {
            if let Err(e) = handle_promote(ctx, cmd).await {
                warn!("Error handling promotion request: {e}");
            }
        }
        _ => {
            debug!("Unknown command !{}", cmd.cmd_name);
        }
    }
}

async fn handle_event(
    handler: &mut HandlerReceiver,
    ctx: &RaceContext<RacetimeState>,
) -> Result<(), RaceTimeBotError> {
    tokio::select! {
        cmd_res = handler.command_rx.recv() => {
            let cmd = cmd_res.ok_or(RaceTimeBotError::HandlerDisconnect)?;
            handle_command(cmd, ctx).await;
            Ok(())
        }
    }
}

async fn handle_race(mut handler: HandlerReceiver, rd: RaceData, ctx: RaceContext<RacetimeState>) {
    debug!("Handle race task started for {rd:?}");
    // first we do new race stuff:
    // check if we've already handled this
    let do_initial_setup = if let Some(msgs) = gethistory(&mut handler, &ctx).await {
        debug!("Got history: {msgs:?}");
        if msgs
            .iter()
            .any(|m| m.bot.as_ref() == Some(&CONFIG.racetime_bot_name))
        {
            send_message(
                "Hello. I just woke up. If you needed me to do something, please ask again.",
                &ctx,
            )
            .await;
            false
        } else {
            true
        }
    } else {
        debug!("No chat history available or whatever");
        true
    };

    if do_initial_setup {
        if let Err(e) = initial_setup(&handler, &ctx).await {
            warn!("Error setting up race room: abandoning I guess? {e}");
            handler.set_end().await;
        }
    }

    // now we wait for events
    loop {
        if let Err(e) = handle_event(&mut handler, &ctx).await {
            warn!("Error handling event. Dropping race. {e}");
            handler.set_end().await;
            break;
        }
    }
}

#[async_trait]
impl RaceHandler<RacetimeState> for Handler {
    async fn should_handle(race_data: &RaceData, state: Arc<RacetimeState>) -> Result<bool, Error> {
        let sh = state.get_bri_id_by_slug(&race_data.slug).await.is_some();
        debug!("should_handle {}? {sh:?}", race_data.slug);
        Ok(sh)
    }

    async fn new(ctx: &RaceContext<RacetimeState>) -> Result<Self, Error> {
        let rd = ctx.data().await;
        let slug = rd.slug.clone();
        let (handler, receiver) = match ctx.global_state.get_bri_id_by_slug(&slug).await {
            Some(id) => Self::new(id, slug),
            None => {
                return Err(Error::Custom(Box::new(RaceTimeBotError::MissingBRI(slug))));
            }
        };
        tokio::spawn(handle_race(receiver, rd.clone(), ctx.clone()));
        Ok(handler)
    }

    async fn command(
        &mut self,
        _ctx: &RaceContext<RacetimeState>,
        cmd_name: String,
        args: Vec<String>,
        is_moderator: bool,
        is_monitor: bool,
        msg: &ChatMessage,
    ) -> Result<(), Error> {
        info!("Got command {cmd_name} with args {args:?}");
        let cmd = Command {
            cmd_name,
            args,
            is_moderator,
            is_monitor,
            msg: msg.clone(),
        };
        if let Err(e) = self.command_tx.send(cmd).await {
            debug!("Error dispatching command to worker thread. Ending race handling. Error: {e}");
            self.set_end().await;
        }
        Ok(())
    }

    async fn should_stop(&mut self, ctx: &RaceContext<RacetimeState>) -> Result<bool, Error> {
        let l = self.should_end.lock().await;
        let should = *l;
        if should {
            Ok(true)
        } else {
            let rd = ctx.data().await;
            match &rd.status.value {
                RaceStatusValue::Finished | RaceStatusValue::Cancelled => Ok(true),
                _ => Ok(false),
            }
        }
    }

    async fn end(self, _ctx: &RaceContext<RacetimeState>) -> Result<(), Error> {
        _ctx.global_state.forget_about_bri(&self.slug).await;
        Ok(())
    }

    async fn chat_history(
        &mut self,
        _ctx: &RaceContext<RacetimeState>,
        msgs: Vec<ChatMessage>,
    ) -> Result<(), Error> {
        if let Err(e) = self.gethistory_tx.send(msgs).await {
            warn!("Error dispatching history: {e} - killing handler");
            return Err(From::from(RaceTimeBotError::WorkerDisconnect));
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
