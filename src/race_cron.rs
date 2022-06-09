use crate::constants::TOKEN_VAR;
use crate::db::get_pool;
use crate::discord::Webhooks;
use crate::models::race::Race;
use crate::models::race_run::{RaceRun, RaceRunState};
use crate::shutdown::Shutdown;
use serenity::http::Http;
use serenity::model::channel::MessageFlags;
use serenity::utils::MessageBuilder;
use sqlx::SqlitePool;
use tokio::sync::broadcast::Receiver;
use tokio::time::Duration;

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
            let mut mb = MessageBuilder::new();
            mb.push("Player: ");
            mb.mention(&run.racer_id());
            mb.push(format!(
                r#"
    Reported time: {}
    Bot time: {}
    VoD URL: {}
    Expected filenames: {}"#,
                run.reported_run_time.as_ref().unwrap_or(&"N/A".to_string()),
                bot_time,
                run.vod.as_ref().unwrap_or(&"N/A".to_string()),
                run.filenames()
                    .map(|f| f.to_string())
                    .unwrap_or("N/A".to_string())
            ));
            mb.build()
        }
        RaceRunState::FORFEIT => {
            let mut mb = MessageBuilder::new();
            mb.push("Player: ");
            mb.mention(&run.racer_id());
            mb.push("    Forfeit");
            mb.build()
        }
        _ => {
            format!("Error: this race wasn't... actually finished... or something")
        }
    }
}

async fn handle_race(mut race: Race, pool: &SqlitePool, webhooks: &Webhooks, http: &Http) {
    let (r1, r2) = match race.get_runs(pool).await {
        Ok(r) => r,
        Err(e) => {
            println!("Error fetching runs for race {}: {}", race.uuid, e);
            // webhooks.
            return;
        }
    };
    if r1.finished() && r2.finished() {
        if let Err(e) = webhooks
            .message_async(&format!(
                r#"Race finished:
{}

{}"#,
                format_finisher(&r1),
                format_finisher(&r2)
            ))
            // .flags(MessageFlags::SUPPRESS_EMBEDS)
            .await
        {
            println!("Error executing webhook: {}", e);
            return;
        }
        race.finish();
        if let Err(e) = race.save(pool).await {
            println!("Error saving finished race: {}", e);
        }
    }
}

async fn sweep(pool: &SqlitePool, http_client: &Http, webhooks: &Webhooks) {
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
        handle_race(race, pool, &webhooks, http_client).await;
    }
}

pub(crate) async fn cron(mut sd: Receiver<Shutdown>, webhooks: Webhooks) {
    let pool = get_pool().await.unwrap();
    let mut intv = tokio::time::interval(Duration::from_secs(60));
    let http_client = Http::new(&dotenv::var(TOKEN_VAR).unwrap());
    loop {
        tokio::select! {
            _ = intv.tick() => {
                sweep(&pool, &http_client, &webhooks).await;
            }
            _sd = sd.recv() => {
                println!("Cron shutting down");
                break;
            }
        }
    }
}
