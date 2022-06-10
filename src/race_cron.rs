use crate::constants::TOKEN_VAR;
use crate::db::get_pool;
use crate::discord::Webhooks;
use crate::models::race::Race;
use crate::models::race_run::{RaceRun, RaceRunState};
use crate::shutdown::Shutdown;
use sqlx::SqlitePool;
use tokio::sync::broadcast::Receiver;
use tokio::time::Duration;
use twilight_http::request::channel::webhook::ExecuteWebhook;
use twilight_mention::Mention;
use twilight_model::channel::message::MessageFlags;
use twilight_model::id::Id;
use twilight_model::id::marker::UserMarker;

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
            let id = Id::<UserMarker>::new(run.racer_id_raw().unwrap());
            format!(
                r#"Player: {}
    Reported time: {}
    Bot time: {}
    VoD URL: {}
    Expected filenames: {}"#,
                id.mention(),
                run.reported_run_time.as_ref().unwrap_or(&"N/A".to_string()),
                bot_time,
                run.vod.as_ref().unwrap_or(&"N/A".to_string()),
                run.filenames()
                    .map(|f| f.to_string())
                    .unwrap_or("N/A".to_string())
            )
        }
        RaceRunState::FORFEIT => {
            format!("Player: {}
    Forfeit",
                run.racer_id_tw().unwrap().mention()
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
            // webhooks.
            return;
        }
    };
    if r1.finished() && r2.finished() {
        let e = webhooks.prepare_execute_async();

        let s = format!(
            r#"Race finished:
{}

{}"#,
            format_finisher(&r1),
            format_finisher(&r2)
        );
        let c = match twilight_validate::message::content(&s) {
            Ok(()) => s,
            Err(e) => {
                format!(
                    "Race finished: {} vs {}",
                    r1.racer_id_tw().unwrap().mention(),
                    r2.racer_id_tw().unwrap().mention(),
                )
            }
        };
        if let Err(e) = webhooks.execute_webhook(
        e.content(&c).unwrap().flags(MessageFlags::SUPPRESS_EMBEDS)
        ).await {
            println!("Error executing webhook: {}", e);
        }
        race.finish();
        if let Err(e) = race.save(pool).await {
            println!("Error saving finished race: {}", e);
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

pub(crate) async fn cron(mut sd: Receiver<Shutdown>, webhooks: Webhooks) {
    let pool = get_pool().await.unwrap();
    let mut intv = tokio::time::interval(Duration::from_secs(60));
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
