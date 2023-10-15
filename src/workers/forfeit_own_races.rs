use crate::discord::discord_state::DiscordState;
use crate::schema::race_runs;
use crate::shutdown::Shutdown;
use diesel::prelude::*;
use log::{debug, info, warn};
use nmg_league_bot::config::CONFIG;
use nmg_league_bot::models::asyncs::race_run::{AsyncRaceRun, RaceRunState};
use std::ops::DerefMut;
use std::sync::Arc;
use tokio::sync::broadcast::Receiver;

async fn sweep(state: &Arc<DiscordState>) {
    let user = match state.cache.current_user() {
        Some(cu) => cu,
        None => {
            warn!("Running without a CurrentUser: self-forfeiting worker sweep noop");
            return;
        }
    };
    let mut cxn = match state.diesel_cxn().await {
        Ok(c) => c,
        Err(e) => {
            warn!("Self-forfeiting worker: error getting db connection: {e}");
            return;
        }
    };
    let active_race_runs: Vec<AsyncRaceRun> = match race_runs::table
        .filter(race_runs::dsl::racer_id.eq(user.id.to_string()))
        // kind of awkward to use CONTACTED here, but that's the state we set new races with ourself to
        .filter(race_runs::dsl::state.eq(RaceRunState::CONTACTED))
        .load::<AsyncRaceRun>(cxn.deref_mut())
    {
        Ok(rs) => rs,
        Err(e) => {
            warn!("Error fetching active race runs to forfeit: {e}");
            return;
        }
    };
    info!("Forfeiting {} races", active_race_runs.len());
    for mut race in active_race_runs {
        race.forfeit();
        if let Err(e) = race.save(cxn.deref_mut()).await {
            warn!("Error forfeiting own race {}: {e}", race.uuid);
        } else {
            debug!("Successfully forfeited race {}", race.uuid);
        }
    }
}

pub(crate) async fn cron(mut sd: Receiver<Shutdown>, state: Arc<DiscordState>) {
    let tick_duration = core::time::Duration::from_secs(CONFIG.cron_tick_seconds);
    info!(
        "Starting self-forfeiting worker: running every {} seconds",
        tick_duration.as_secs()
    );
    let mut intv = tokio::time::interval(tick_duration);
    loop {
        tokio::select! {
            _ = intv.tick() => {
                sweep(&state).await;
            }
            _sd = sd.recv() => {
                info!("self-forfeiting worker shutting down");
                break;
            }
        }
    }
}
