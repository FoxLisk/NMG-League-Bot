use crate::discord::discord_state::DiscordState;
use crate::shutdown::Shutdown;
use chrono::{Duration, Utc};
use log::{debug, info, warn};
use nmg_league_bot::models::bracket_race_infos::BracketRaceInfoId;
use nmg_league_bot::models::season::Season;
use nmg_league_bot::NMGLeagueBotError;
use std::ops::DerefMut;
use std::sync::Arc;
use tokio::sync::broadcast::Receiver;
use tokio::sync::mpsc::Sender;

async fn run_once(
    upcoming_race_tx: &Sender<BracketRaceInfoId>,
    state: &Arc<DiscordState>,
) -> Result<(), NMGLeagueBotError> {
    let mut conn = state.diesel_cxn().await?;

    let cur_szn = match Season::get_active_season(conn.deref_mut())? {
        Some(szn) => szn,
        None => return Ok(()),
    };
    let when = Utc::now() + Duration::minutes(30);
    // it would be pretty weird to have a finished race thats scheduled for the future but i'm not
    // very confident that it's impossible >_<
    let upcoming =
        cur_szn.get_unfinished_races_starting_before(when.timestamp(), conn.deref_mut())?;
    for (bri, _) in upcoming {
        if bri.racetime_gg_url.is_none() {
            let id = BracketRaceInfoId(bri.id);
            if let Err(e) = upcoming_race_tx.send(id.clone()).await {
                warn!("Error sending upcoming race: {e}");
            }
            #[cfg(feature = "testing")]
            {
                debug!("Double sending race to test idempotency");
                upcoming_race_tx.send(id.clone()).await.ok();
            }
        }
    }
    Ok(())
}

pub async fn cron(
    upcoming_race_tx: Sender<BracketRaceInfoId>,
    mut sd: Receiver<Shutdown>,
    state: Arc<DiscordState>,
) {
    let mut interval = tokio::time::interval(core::time::Duration::from_secs(60));
    info!("Starting upcoming_races_worker...");
    loop {
        tokio::select! {
            _ = interval.tick() => {
            },
            _ = sd.recv() => {
                info!("Shutting down upcoming_races_worker");
                break;
            }
        }
        debug!("upcoming_races_worker scan starting");
        if let Err(e) = run_once(&upcoming_race_tx, &state).await {
            warn!("Error running upcoming_races_worker loop: {e}");
        }
    }
}
