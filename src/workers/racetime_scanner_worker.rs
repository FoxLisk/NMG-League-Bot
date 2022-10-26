use crate::constants::CRON_TICKS_VAR;
use crate::discord::discord_state::DiscordState;
use crate::schema::players;
use crate::workers::get_tick_duration;
use crate::Shutdown;
use bb8::RunError;
use diesel::prelude::*;
use nmg_league_bot::models::bracket_race_infos::BracketRaceInfo;
use nmg_league_bot::models::bracket_races::{BracketRace, BracketRaceStateError};
use nmg_league_bot::models::player::Player;
use nmg_league_bot::racetime_types::{Entrant, PlayerResultError, Races, RacetimeRace};
use nmg_league_bot::worker_funcs::{
    get_races_that_should_be_finishing_soon, interesting_race, races_by_player_rtgg,
    trigger_race_finish,
};
use racetime_api::client::RacetimeClient;
use racetime_api::endpoint::Query;
use racetime_api::endpoints::{PastCategoryRaces, PastCategoryRacesBuilder};
use racetime_api::err::RacetimeError;
use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::broadcast::Receiver;
use nmg_league_bot::constants::RACETIME_TICK_SECS;

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
    let bracket_races = get_races_that_should_be_finishing_soon(cxn.deref_mut())?;

    // *shrug*
    // it's like 40 rows
    let all_players: Vec<Player> = players::table.load(cxn.deref_mut())?;
    let interesting_rtgg_ids = races_by_player_rtgg(&all_players, &bracket_races);

    let recent_races: PastCategoryRaces = PastCategoryRacesBuilder::default()
        .show_entrants(true)
        .category("alttp")
        .build()?;

    let finished_races: Races = recent_races.query(racetime_client).await?;

    for race in finished_races.races {
        if let Err(e) = maybe_do_race_stuff(race, &interesting_rtgg_ids, state).await {
            println!("Error handling a race: {}", e);
        }
    }
    Ok(())
}

async fn maybe_do_race_stuff(
    mut race: RacetimeRace,
    bracket_races: &HashMap<String, (&BracketRaceInfo, &BracketRace, &Player, &Player)>,
    state: &Arc<DiscordState>,
) -> Result<(), ScanError> {
    if let Some((bri, br, (p1, e1), (p2, e2))) = interesting_race(&mut race, bracket_races) {
        // this is awful, i hate doing it this way, i'm just tired of thinking about this
        let mutable_br = br.clone();
        let mut mutable_bri = bri.clone();
        mutable_bri.racetime_gg_url = Some(race.url.clone());
        let p1r = e1.result()?;
        let p2r = e2.result()?;
        let mut conn = state.diesel_cxn().await?;
        if let Err(e) = trigger_race_finish(
            mutable_br,
            &mutable_bri,
            (p1, p1r),
            (p2, p2r),
            conn.deref_mut(),
            Some(&state.client),
            &state.channel_config,
        )
        .await
        {
            println!("Error triggering race finish: {}", e);
        }
    }
    Ok(())
}

pub async fn cron(mut sd: Receiver<Shutdown>, state: Arc<DiscordState>) {
    let tick_duration = get_tick_duration(RACETIME_TICK_SECS);
    println!(
        "Starting racetime scanner worker: running every {} seconds",
        tick_duration.as_secs()
    );
    let mut intv = tokio::time::interval(tick_duration);
    let client = RacetimeClient::new().unwrap();
    loop {
        tokio::select! {
            _ = intv.tick() => {
                println!("Racetime scan starting...");
                if let Err(e) = scan(&state, &client).await {
                    println!("Error running racetime scan: {}", e);
                }
            }
            _sd = sd.recv() => {
                println!("racetime scanner worker shutting down");
                break;
            }
        }
    }
}
