use crate::constants::CRON_TICKS_VAR;
use crate::db::get_pool;
use crate::discord::Webhooks;
use crate::models::race::Race;
use crate::models::race_run::{RaceRun, RaceRunState};
use crate::shutdown::Shutdown;
use sqlx::SqlitePool;
use tokio::sync::broadcast::Receiver;
use tokio::time::Duration;
use twilight_mention::Mention;
use twilight_model::channel::message::MessageFlags;
use twilight_model::id::marker::UserMarker;
use twilight_model::id::Id;

fn format_secs(secs: u64) -> String {
    let mins = secs / 60;
    let hours = mins / 60;
    if hours > 0 {
        format!(
            "{hours}:{mins:02}:{secs:02}",
            hours = hours,
            mins = mins % 60,
            secs = secs % 60 % 60
        )
    } else {
        format!(
            "{mins:02}:{secs:02}",
            mins = mins % 60,
            secs = secs % 60 % 60
        )
    }
}

fn format_finisher(run: &RaceRun) -> String {
    match run.state {
        RaceRunState::VOD_SUBMITTED => {
            let bot_time = match (run.run_started, run.run_finished) {
                (Some(start), Some(finish)) => format_secs((finish - start) as u64),
                _ => "N/A".to_string(),
            };
            let p = run.racer_id().map(|id| id.mention().to_string()).unwrap_or("Error finding user".to_string());
            format!(
                r#"Player: {}
    Reported time: {}
    Bot time: {}
    VoD URL: {}
    Expected filenames: {}"#,
                p,
                run.reported_run_time.as_ref().unwrap_or(&"N/A".to_string()),
                bot_time,
                run.vod.as_ref().unwrap_or(&"N/A".to_string()),
                run.filenames()
                    .map(|f| f.to_string())
                    .unwrap_or("N/A".to_string())
            )
        }
        RaceRunState::FORFEIT => {
            format!(
                "Player: {}
    Forfeit",
                run.racer_id().unwrap().mention()
            )
        }
        _ => {
            format!("Error: this race wasn't... actually finished... or something")
        }
    }
}

async fn handle_race(mut race: Race, pool: &SqlitePool, webhooks: &Webhooks) {
    let (r1, r2) = match race.get_runs(pool).await {
        Ok(r) => r,
        Err(e) => {
            println!("Error fetching runs for race {}: {}", race.uuid, e);
            return;
        }
    };
    if r1.finished() && r2.finished() {
        let s = format!(
            r#"Race {} finished:
{}

{}"#,
            race.uuid,
            format_finisher(&r1),
            format_finisher(&r2)
        );
        let c = match twilight_validate::message::content(&s) {
            Ok(()) => s,
            Err(_e) => {
                format!(
                    "Race finished: {} vs {}",
                    r1.racer_id().unwrap().mention(),
                    r2.racer_id().unwrap().mention(),
                )
            }
        };
        // N.B. this level of error handling is realistically unnecessary
        let ew = match webhooks.prepare_execute_async().content(&c) {
            Ok(ex) => ex,
            Err(mve) => {
                match webhooks.prepare_execute_async().content("A race finished but I can't tell you which one for some reason. Check the logs") {
                    Ok(ex) => ex,
                    Err(mve) => {
                        println!("Error reporting race: {}", c);
                        return;
                    }
                }
            }
        }.flags(MessageFlags::SUPPRESS_EMBEDS);

        if let Err(e) = webhooks
            .execute_webhook(ew)
            .await
        {
            println!("Error executing webhook: {}", e);
        }
        race.finish();
        if let Err(e) = race.save(pool).await {
            println!("Error saving finished race: {}", e);
            // TODO: report this, too, probably
        }
    }
}

async fn sweep(pool: &SqlitePool, webhooks: &Webhooks) {
    println!("Sweep...");
    let active_races = match Race::active_races(pool).await {
        Ok(rs) => rs,
        Err(e) => {
            println!("Error fetching active races: {}", e);
            return;
        }
    };
    println!("Handling {} races", active_races.len());
    for race in active_races {
        handle_race(race, pool, &webhooks).await;
    }
}

fn get_tick_duration() -> Duration {
    Duration::from_secs(
        std::env::var(CRON_TICKS_VAR)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60),
    )
}

pub(crate) async fn cron(mut sd: Receiver<Shutdown>, webhooks: Webhooks) {
    let pool = get_pool().await.unwrap();

    let tick_duration = get_tick_duration();
    println!(
        "Starting cron: running every {} seconds",
        tick_duration.as_secs()
    );
    let mut intv = tokio::time::interval(tick_duration);
    loop {
        tokio::select! {
            _ = intv.tick() => {
                sweep(&pool, &webhooks).await;
            }
            _sd = sd.recv() => {
                println!("Cron shutting down");
                break;
            }
        }
    }
}
