//! Worker to sync Discord event status with DB race scheduling information
//!
//! This periodically scans the DB for interesting races, compares them with the
//! set of scheduled events in the Discord, and resolves discrepancies by creating or
//! updating events as necessary.
//!
//! This means that events will get reliably updated on any relevant change, including
//! small things like players changing their nicknames.
use std::{collections::HashMap, ops::DerefMut, sync::Arc, time::Duration};

use bb8::Pool;
use bot::{GuildEventConfig, HelperBot};
use chrono::Utc;
use diesel::SqliteConnection;
use itertools::Itertools as _;
use log::{info, warn};
use nmg_league_bot::{
    config::CONFIG,
    db::DieselConnectionManager,
    models::{
        bracket_race_infos::BracketRaceInfo,
        bracket_races::{BracketRace, BracketRaceState},
        brackets::Bracket,
        player::Player,
        race_events::{NewRaceEvent, RaceEvent},
        season::SeasonState,
    },
    schema::{self},
    NMGLeagueBotError, RaceEventError,
};
use tokio::sync::broadcast::Receiver;
use twilight_model::{
    guild::scheduled_event::{GuildScheduledEvent, Status},
    id::{
        marker::{GuildMarker, ScheduledEventMarker},
        Id,
    },
    util::Timestamp,
};

use crate::discord::discord_state::DiscordOperations;
use crate::{
    discord::{comm_ids_and_names, discord_state::DiscordState},
    shutdown::Shutdown,
};

mod bot;

/// describe the fields that should be populated in an event
#[derive(Debug)]
struct RaceEventContent {
    name: String,
    location: String,
    /// races without commentators have no description
    description: Option<String>,
    /// use [self.start_timestamp()] instead of this
    start: i64,
    /// use [self.end_timestamp()] instead of this
    end: i64,
}

impl RaceEventContent {
    fn start_timestamp(&self) -> Result<Timestamp, NMGLeagueBotError> {
        Timestamp::from_secs(self.start).map_err(From::from)
    }

    fn end_timestamp(&self) -> Result<Timestamp, NMGLeagueBotError> {
        Timestamp::from_secs(self.end).map_err(From::from)
    }
}

/// Information about what an event should look like
#[derive(Debug)]
enum RaceEventContentAndStatus {
    /// indicates that, if there's an event, we should end or cancel it, and if there's not we don't create one
    NoEvent,
    /// indicates that we should create or update an existing event to the state defined here
    Event(RaceEventContent),
}

#[derive(Debug)]
struct RaceInfoBundle {
    race: BracketRace,
    bri: BracketRaceInfo,
    status: RaceEventContentAndStatus,
}

pub async fn launch(
    mut sd: Receiver<Shutdown>,
    state: Arc<DiscordState>,
    pool: Pool<DieselConnectionManager>,
) {
    let mut intv = tokio::time::interval(Duration::from_secs(CONFIG.race_event_worker_tick_secs));
    let bot = Arc::new(HelperBot::new(pool));
    tokio::spawn(HelperBot::run(bot.clone(), sd.resubscribe()));

    loop {
        tokio::select! {
            _sd = sd.recv() => {
                break;
            }
            _ = intv.tick() => {

                let t = tokio::time::Instant::now();
                if let Err(e) = sync_race_status(&state, &bot).await {
                    warn!("Error syncing race events: {e}");
                }
                let t2 = tokio::time::Instant::now() - t;
                info!("race event worker loop took {}ms", t2.as_millis());
            }
        }
    }
    warn!("Race event worker quit");
}

async fn sync_race_status(
    state: &Arc<DiscordState>,
    helper_bot: &Arc<HelperBot>,
) -> Result<(), NMGLeagueBotError> {
    let mut conn_o = state.diesel_cxn().await?;
    let conn = conn_o.deref_mut();
    // grab the current state of races: this is the scheduled races + the status of what their events
    // should look like. this will be the same across guilds, so we grab it up front
    let race_infos = get_season_race_info(state, conn).await?;
    let bri_ids = race_infos
        .iter()
        .map(|bundle| bundle.bri.get_id())
        .collect::<Vec<_>>();

    // { guild_id: { bracket_race_info_id: RaceEvent }}
    // this is every RaceEvent, grouped by guild, so that in a moment we know what existing state to
    // look for
    let mut race_events_by_guild = RaceEvent::get_for_bri_ids(&bri_ids, conn)?
        .into_iter()
        .group_by(|re| re.guild_id.clone())
        .into_iter()
        .map(|(guild_id, race_events)| {
            (
                guild_id,
                race_events
                    .map(|re| (re.bracket_race_info_id, re))
                    .collect(),
            )
        })
        .collect::<HashMap<_, _>>();

    // for each guild we're syncing events to, do the syncing
    // the list of guilds to sync is just "whichever ones the bot is currently added to"
    for gev in helper_bot.guild_event_configs() {
        let race_events_by_bri_id = race_events_by_guild
            .remove(&gev.guild_id.to_string())
            .unwrap_or(HashMap::new());

        if let Err(e) =
            sync_events_in_a_guild(gev, helper_bot, &race_infos, race_events_by_bri_id, conn).await
        {
            warn!("Error syncing events in a guild: {e}");
        }
    }
    Ok(())
}

/// this looks at `race_infos`, which describe what the events for current races *should* look like,
/// and compares it with the actually-existing events in a guild, and tries to reconcile the differences.
///
/// The `race_events_by_bri_id` allow us to track existing events for races through changes
// XXX probably we could include race id #s in the event info somewhere and use that to track them over time...
// that would be very vulnerable to users modifying events, though.
async fn sync_events_in_a_guild(
    guild_event_config: GuildEventConfig,
    helper_bot: &Arc<HelperBot>,
    race_infos: &[RaceInfoBundle],
    mut race_events_by_bri_id: HashMap<i32, RaceEvent>,
    conn: &mut SqliteConnection,
) -> Result<(), NMGLeagueBotError> {
    let mut existing_events =
        get_existing_events_by_id(guild_event_config.guild_id, helper_bot).await?;

    for bundle in race_infos {
        let race_event = race_events_by_bri_id.remove(&bundle.bri.id);
        let existing_event = if let Some(gse_id) = race_event
            .as_ref()
            .map(|re| re.get_scheduled_event_id())
            .flatten()
        {
            existing_events.remove(&gse_id)
        } else {
            None
        };

        let empty_status = RaceEventContentAndStatus::NoEvent;

        let desired_status = if guild_event_config.should_sync_race(&bundle.race) {
            &bundle.status
        } else {
            // if the race isn't meant to be synced to this discord, we just pass in NoEvent to indicate that it should be
            // either ignored or deleted
            &empty_status
        };

        // since this uses *actual* events that match the event_id on the RaceEvent,
        // this will have some weird behaviours if the DB is out of sync with the events -
        // i think if we created an event and then somehow changed BRI.scheduled_event_id to a wrong value,
        // we'd create a new duplicated event with the same info and set the BRI's event id to that value.
        //
        // in practice idk how that would ever happen.
        // it has the nice side effect that it handles testing cases nicely when I copy the DB from prod lol

        match update_discord_events(
            guild_event_config.guild_id,
            desired_status,
            existing_event.as_ref(),
            helper_bot,
        )
        .await
        {
            Ok(Some(e)) => {
                if let Some(mut re) = race_event {
                    re.set_scheduled_event_id(e.id);
                    if let Err(e) = re.update(conn) {
                        warn!(
                            "Error updating RaceEvent {} after creating event: {e}",
                            re.id
                        );
                    }
                } else {
                    let nre = NewRaceEvent::new(guild_event_config.guild_id, &bundle.bri, e.id);
                    if let Err(e) = nre.save(conn) {
                        warn!("Error creating new RaceEvent after creating event: {e}");
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                warn!(
                    "Error managing event for race {}: {e} - existing event is {existing_event:?}",
                    bundle.race.id
                );
            }
        }
    }
    Ok(())
}

async fn get_existing_events_by_id(
    guild_id: Id<GuildMarker>,
    bot: &Arc<HelperBot>,
) -> Result<HashMap<Id<ScheduledEventMarker>, GuildScheduledEvent>, NMGLeagueBotError> {
    let events = bot.get_guild_scheduled_events(guild_id).await?;
    Ok(events
        .into_iter()
        .map(|gse| (gse.id, gse))
        .collect::<HashMap<_, _>>())
}

/// does any discord updates that are necessary (creating or updating events)
///
/// returns a GuildScheduledEvent if one is created (for persistence reasons)
async fn update_discord_events(
    guild_id: Id<GuildMarker>,
    new_status: &RaceEventContentAndStatus,
    existing_event: Option<&GuildScheduledEvent>,
    helper_bot: &Arc<HelperBot>,
) -> Result<Option<GuildScheduledEvent>, NMGLeagueBotError> {
    match (existing_event, new_status) {
        (Some(event), RaceEventContentAndStatus::NoEvent) => {
            let new_event_status = match event.status {
                // active races must be completed; all others must be cancelled
                Status::Active => Status::Completed,
                _ => Status::Cancelled,
            };
            // race is over; end event (if it's not ended)
            let e = helper_bot
                .update_scheduled_event(guild_id, event.id)
                .status(new_event_status)
                .await?;
            Ok(Some(e.model().await?))
        }
        (None, RaceEventContentAndStatus::NoEvent) => {
            // race is over, event is already ended; nothing to do
            Ok(None)
        }
        (Some(event), RaceEventContentAndStatus::Event(new_status)) => {
            // race is scheduled; compare details and update if necessary
            if events_match(&event, &new_status)? {
                Ok(None)
            } else {
                helper_bot
                    .update_scheduled_event(guild_id, event.id)
                    .description(new_status.description.as_ref().map(|s| s.as_str()))?
                    .name(&new_status.name)?
                    .scheduled_start_time(&new_status.start_timestamp()?)
                    .scheduled_end_time(Some(&new_status.end_timestamp()?))
                    .await?;
                // it's not really interesting that an event has been updated
                Ok(None)
            }
        }
        (None, RaceEventContentAndStatus::Event(new_status)) => {
            if new_status.start < Utc::now().timestamp() {
                // a race was scheduled for the past somehow - creating events in the past gives us a discord API error, so let's not do that
                return Ok(None);
            }
            // race has been scheduled but there's no event yet; create one
            let resp = helper_bot
                .create_scheduled_event(guild_id)
                .external(
                    &new_status.name,
                    &new_status.location,
                    &new_status.start_timestamp()?,
                    &new_status.end_timestamp()?,
                )?
                .await?;
            Ok(Some(resp.model().await?))
        }
    }
}

fn events_match(
    existing_event: &GuildScheduledEvent,
    new_status: &RaceEventContent,
) -> Result<bool, NMGLeagueBotError> {
    if existing_event.name == new_status.name
        && existing_event.description == new_status.description
        && existing_event.scheduled_start_time.as_secs() == new_status.start
        && existing_event.scheduled_end_time.map(|t| t.as_secs()) == Some(new_status.end)
    {
        if let Some(emd) = &existing_event.entity_metadata {
            if let Some(l) = &emd.location {
                if *l == new_status.location {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn multistream_link(p1: &Player, p2: &Player) -> String {
    format!(
        "https://multistre.am/{}/{}/layout4/",
        p1.twitch_user_login
            .clone()
            .unwrap_or("<unknown>".to_string()),
        p2.twitch_user_login
            .clone()
            .unwrap_or("<unknown>".to_string()),
    )
}

/// very unfortunate but this seems to need the discord state in order to get comm names
/// from the main discord for comms who aren't also signed up as players
async fn get_event_content<D: DiscordOperations>(
    race: &BracketRace,
    bri: &BracketRaceInfo,
    bracket: &Bracket,
    players: &HashMap<i32, Player>,
    state: &Arc<D>,
    conn: &mut SqliteConnection,
) -> Result<RaceEventContentAndStatus, NMGLeagueBotError> {
    if race.state()? != BracketRaceState::Scheduled {
        return Ok(RaceEventContentAndStatus::NoEvent);
    }
    let when = bri.scheduled().ok_or(NMGLeagueBotError::MissingTimestamp)?;

    let p1 = players
        .get(&race.player_1_id)
        .ok_or(RaceEventError::MissingPlayer(race.player_1_id))?;
    let p2 = players
        .get(&race.player_2_id)
        .ok_or(RaceEventError::MissingPlayer(race.player_2_id))?;

    let event_name = format!("{}: {} vs {}", bracket.name, p1.name, p2.name);

    let event_location = match &bri.restream_channel {
        Some(s) => s.to_string(),
        None => multistream_link(p1, p2),
    };

    // NOTE: as it stands, this is going to be a behavioural change - we are going to put commentator names in
    // races that have not had commentators decided yet!
    //
    // but see https://github.com/FoxLisk/NMG-League-Bot/issues/152
    let description = {
        let comm_info = comm_ids_and_names(&bri, state, conn).await?;
        if comm_info.is_empty() {
            None
        } else {
            Some(format!(
                "with comms by {}",
                comm_info.iter().map(|(_, n)| n).join(" and ")
            ))
        }
    };

    let start = when.timestamp();
    let end = (when.clone() + chrono::Duration::minutes(100)).timestamp();

    Ok(RaceEventContentAndStatus::Event(RaceEventContent {
        name: event_name,
        location: event_location,
        description,
        start,
        end,
    }))
}

/// retrieves every race that has a BRI in the current season (i.e. every race that has been scheduled)
///
/// This includes completed races and races from completed rounds.
async fn get_season_race_info(
    state: &Arc<DiscordState>,
    conn: &mut SqliteConnection,
) -> Result<Vec<RaceInfoBundle>, NMGLeagueBotError> {
    use diesel::prelude::*;
    let season_started_state = serde_json::to_string(&SeasonState::Started)?;
    // i think that "every race that already has a BRI and is in the current season" is the correct
    // set to look at
    // this will pick up prior round races that we'll have to look at repeatedly but i think that's not a big deal
    // and we have to pick up recently-completed races to close out events

    // N.B. if it mattered it might be worth testing if it's faster to do 1 query and pull the Bracket table every time, or
    // do a separate query for the brackets like we are doing for players

    let races: Vec<(BracketRace, BracketRaceInfo, Bracket)> = schema::bracket_race_infos::table
        .inner_join(
            schema::bracket_races::table
                .inner_join(schema::brackets::table.inner_join(schema::seasons::table)),
        )
        .select((
            BracketRace::as_select(),
            BracketRaceInfo::as_select(),
            Bracket::as_select(),
        ))
        .filter(schema::seasons::dsl::state.eq(season_started_state))
        .load(conn)?;

    let players = Player::by_id(
        Some(
            races
                .iter()
                .map(|(race, _, _)| [race.player_1_id, race.player_2_id])
                .flatten()
                .collect::<_>(),
        ),
        conn,
    )?;

    let mut infos = Vec::with_capacity(races.len());
    for (race, bri, bracket) in races {
        let status = get_event_content(&race, &bri, &bracket, &players, state, conn).await?;
        infos.push(RaceInfoBundle { race, bri, status });
    }

    Ok(infos)
}
