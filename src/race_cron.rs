use crate::constants::{TOKEN_VAR, WEBHOOK_VAR};
use crate::models::race::Race;
use crate::models::race_run::{RaceRun, RaceRunState};
use serenity::http::Http;
use serenity::model::webhook::Webhook;
use serenity::utils::MessageBuilder;
use sqlx::SqlitePool;
use tokio::time::Duration;
use crate::shutdown::Shutdown;
use tokio::sync::broadcast::Receiver;

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
                run.filenames().map(|f|f.to_string()).unwrap_or("N/A".to_string())
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

async fn handle_race(mut race: Race, pool: &SqlitePool, webhook: &Webhook, http: &Http) {
    let (r1, r2) = match race.get_runs(pool).await {
        Ok(r) => r,
        Err(e) => {
            println!("Error fetching runs for race {}: {}", race.uuid, e);
            return;
        }
    };
    if r1.finished() && r2.finished() {
        if let Err(e) = webhook
            .execute(http, false, |ew| {
                ew.content(format!(
                    r#"Race finished:
{}

{}"#,
                    format_finisher(&r1),
                    format_finisher(&r2)
                ))
            })
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

async fn sweep(pool: &SqlitePool) {
    let http_client = Http::new(&dotenv::var(TOKEN_VAR).unwrap());
    let webhook = http_client
        .get_webhook_from_url(&dotenv::var(WEBHOOK_VAR).unwrap())
        .await
        .unwrap();
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
        handle_race(race, pool, &webhook, &http_client).await;
    }
}

pub(crate) async fn cron(pool: SqlitePool, mut sd: Receiver<Shutdown>) {
    let mut intv = tokio::time::interval(Duration::from_secs(60));
    loop {
        tokio::select! {
            _ = intv.tick() => {
                sweep(&pool).await;
            }
            _sd = sd.recv() => {
                println!("Cron shutting down");
            }
        }
    }
}
