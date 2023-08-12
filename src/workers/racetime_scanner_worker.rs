use crate::discord::discord_state::DiscordState;
use crate::schema::players;
use crate::Shutdown;
use bb8::RunError;
use diesel::prelude::*;
use itertools::Itertools;
use log::{debug, info, warn};
use nmg_league_bot::config::CONFIG;
use nmg_league_bot::models::bracket_race_infos::BracketRaceInfo;
use nmg_league_bot::models::bracket_races::{BracketRace, BracketRaceStateError};
use nmg_league_bot::models::player::Player;
use nmg_league_bot::models::season::Season;
use nmg_league_bot::racetime_types::{PlayerResultError, Races, RacetimeRace};
use nmg_league_bot::worker_funcs::{
    interesting_race, races_by_player_rtgg, trigger_race_finish, RaceFinishOptions,
};
use racetime_api::client::RacetimeClient;
use racetime_api::endpoint::Query;
use racetime_api::endpoints::{PastCategoryRaces, PastCategoryRacesBuilder};
use racetime_api::err::RacetimeError;
use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::broadcast::Receiver;

#[derive(Error, Debug)]
enum ScanError {
    #[error("Error getting DB connection: {0}")]
    ConnectionError(#[from] RunError<ConnectionError>),

    #[error("Error running DB query: {0}")]
    DatabaseError(#[from] diesel::result::Error),

    #[error("Totally unrealistic serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Totally unrealistic query builder error: {0}")]
    BuilderError(#[from] racetime_api::endpoints::PastCategoryRacesBuilderError),

    #[error("Racetime API error: {0}")]
    RacetimeError(#[from] RacetimeError),

    #[error("Error determing player's race result: {0}")]
    PlayerResultError(#[from] PlayerResultError),

    #[error("Error updating bracket race state: {0}")]
    BracketRaceStateError(#[from] BracketRaceStateError),
}

async fn scan(
    state: &Arc<DiscordState>,
    racetime_client: &RacetimeClient,
) -> Result<(), ScanError> {
    let mut cxn = state.diesel_cxn().await?;
    let season = match Season::get_active_season(cxn.deref_mut())? {
        Some(s) => s,
        None => {
            debug!("No active season.");
            return Ok(());
        }
    };
    let bracket_races = season.get_races_that_should_be_finishing_soon(cxn.deref_mut())?;
    debug!("Looking for status on {} races", bracket_races.len());
    if bracket_races.is_empty() {
        // don't do all the racetime scanning stuff if there's nothing to look for
        return Ok(());
    }

    // *shrug*
    // it's like 40 rows
    let all_players: Vec<Player> = players::table.load(cxn.deref_mut())?;
    let interesting_rtgg_ids = races_by_player_rtgg(&all_players, &bracket_races);
    let rtgg_ids_str = interesting_rtgg_ids.keys().join(", ");
    debug!("Interesting rtgg ids that we're looking for: {rtgg_ids_str}");
    if interesting_rtgg_ids.is_empty() {
        // there's no guarantee that everyone has their racetime username set. they *should* but that's an "invariant"
        // managed by me hopefully noticing and sending discord messages, so we can be in this state.
        return Ok(());
    }
    let recent_races: PastCategoryRaces = PastCategoryRacesBuilder::default()
        .show_entrants(true)
        .category(&season.rtgg_category_name)
        .build()?;

    let finished_races: Races = recent_races.query(racetime_client).await?;

    for race in finished_races.races {
        debug!("Checking race {race:?}");
        if let Err(e) = maybe_do_race_stuff(race, &interesting_rtgg_ids, &season, state).await {
            warn!("Error handling a race: {}", e);
        }
    }
    Ok(())
}

async fn maybe_do_race_stuff(
    mut race: RacetimeRace,
    bracket_races: &HashMap<String, (&BracketRaceInfo, &BracketRace, &Player, &Player)>,
    season: &Season,
    state: &Arc<DiscordState>,
) -> Result<(), ScanError> {
    if let Some((bri, br, (p1, e1), (p2, e2))) = interesting_race(&mut race, bracket_races, season)
    {
        // this is awful, i hate doing it this way, i'm just tired of thinking about this
        let mutable_br = br.clone();
        let mut mutable_bri = bri.clone();
        mutable_bri.racetime_gg_url = Some(format!("https://racetime.gg{}", race.url));
        let p1r = e1.result()?;
        let p2r = e2.result()?;
        let mut conn = state.diesel_cxn().await?;
        let opts = RaceFinishOptions {
            bracket_race: mutable_br,
            info: mutable_bri,
            player_1: p1.clone(),
            player_1_result: p1r,
            player_2: p2.clone(),
            player_2_result: p2r,
            channel_id: state.channel_config.match_results,
            force_update: false,
        };
        if let Err(e) = trigger_race_finish(
            opts,
            conn.deref_mut(),
            Some(&state.client),
            Some(state.guild_id()),
            &state.channel_config,
        )
        .await
        {
            warn!("Error triggering race finish: {}", e);
        }
    }
    Ok(())
}

pub async fn cron(mut sd: Receiver<Shutdown>, state: Arc<DiscordState>) {
    let tick_duration = Duration::from_secs(CONFIG.racetime_tick_secs);
    info!(
        "Starting racetime scanner worker: running every {} seconds",
        tick_duration.as_secs()
    );
    let mut intv = tokio::time::interval(tick_duration);
    let client = RacetimeClient::new().unwrap();
    loop {
        tokio::select! {
            _ = intv.tick() => {
                debug!("Racetime scan starting...");
                if let Err(e) = scan(&state, &client).await {
                    warn!("Error running racetime scan: {}", e);
                }
            }
            _sd = sd.recv() => {
                info!("racetime scanner worker shutting down");
                break;
            }
        }
    }
}
