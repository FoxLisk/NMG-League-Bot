use crate::constants::{CRON_TICKS_VAR, RACETIME_TICK_SECS};
use crate::discord::discord_state::DiscordState;
use crate::discord::{notify_racer, Webhooks};
use nmg_league_bot::models::race::{Race, RaceState};
use nmg_league_bot::models::race_run::{RaceRun, RaceRunState};
use crate::schema::races;
use crate::shutdown::Shutdown;
use crate::utils::{env_default, format_hms};
use diesel::prelude::*;
use std::ops::DerefMut;
use std::sync::Arc;
use tokio::sync::broadcast::Receiver;
use tokio::time::Duration;
use twilight_mention::Mention;
use twilight_model::channel::message::MessageFlags;
use crate::workers;

fn format_finisher(run: &RaceRun) -> String {
    match run.state {
        RaceRunState::VOD_SUBMITTED => {
            let bot_time = match (run.run_started, run.run_finished) {
                (Some(start), Some(finish)) => format_hms((finish - start) as u64),
                _ => "N/A".to_string(),
            };
            let p = run
                .racer_id()
                .map(|id| id.mention().to_string())
                .unwrap_or("Error finding user".to_string());
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

trait Fold<T> {
    fn fold(self) -> T;
}

impl<T> Fold<T> for Result<T, T> {
    fn fold(self) -> T {
        match self {
            Ok(t) => t,
            Err(t) => t,
        }
    }
}

async fn handle_race(mut race: Race, state: &Arc<DiscordState>, webhooks: &Webhooks) {
    let mut conn = match state.diesel_cxn().await {
        Ok(c) => c,
        Err(e) => {
            println!("Error getting diesel connection: {:?}", e);
            return;
        }
    };
    let (mut r1, mut r2) = match race.get_runs(&mut conn).await {
        Ok(r) => r,
        Err(e) => {
            println!("Error fetching runs for race {}: {}", race.uuid, e);
            race.abandon();
            if let Err(e) = race.save(&mut conn).await {
                println!("Error abandoning race {}: {}", race.uuid, e);
            }
            return;
        }
    };
    if r1.is_finished() && r2.is_finished() {
        handle_finished_race(&mut race, &r1, &r2, &mut conn, webhooks).await
    } else {
        let mut msgs = Vec::with_capacity(2);
        if r1.state.is_created() {
            let name = r1.racer_id().map(|uid| uid.mention().to_string()).fold();
            if let Err(e) = notify_racer(&mut r1, &race, state).await {
                println!("Error notifying {}: {}", name, e);
            } else {
                msgs.push(format!("Successfully contacted {}", name));
            }
        }
        if r2.state.is_created() {
            let name = r2.racer_id().map(|uid| uid.mention().to_string()).fold();
            if let Err(e) = notify_racer(&mut r2, &race, state).await {
                println!("Error notifying {}: {}", name, e);
            } else {
                msgs.push(format!("Successfully contacted {}", name));
            }
        }
        if !msgs.is_empty() {
            let msg = format!("Race update: {}", msgs.join(", "));
            if let Err(e) = webhooks.message_async(&msg).await {
                println!("Error notifying admins about race update: Tried to tell them {}, encountered error {}", msg, e);
            }
        }
    }
}

async fn handle_finished_race(
    race: &mut Race,
    r1: &RaceRun,
    r2: &RaceRun,
    conn: &mut SqliteConnection,
    webhooks: &Webhooks,
) {
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
            println!("Message validation error sending {}: {}", c, mve);
            match webhooks.prepare_execute_async().content(
                "A race finished but I can't tell you which one for some reason. Check the logs",
            ) {
                Ok(ex) => ex,
                Err(mve) => {
                    println!("Error reporting race {}: {}", race.uuid, mve);
                    return;
                }
            }
        }
    }
    .flags(MessageFlags::SUPPRESS_EMBEDS);

    if let Err(e) = webhooks.execute_webhook(ew).await {
        println!("Error executing webhook: {}", e);
    }
    race.finish();
    if let Err(e) = race.save(conn).await {
        println!("Error saving finished race: {}", e);
        // TODO: report this, too, probably
    }
}

async fn sweep(state: &Arc<DiscordState>, webhooks: &Webhooks) {
    println!("Sweep...");
    let active_races: Vec<Race> = match state.diesel_cxn().await.map(|mut cxn| {
        races::table
            .filter(races::dsl::state.eq(RaceState::CREATED))
            .load::<Race>(cxn.deref_mut())
    }) {
        Ok(qr) => match qr {
            Ok(rs) => rs,
            Err(e) => {
                println!("Error fetching active races: {}", e);
                return;
            }
        },
        Err(e) => {
            println!("Error fetching active races: {}", e);
            return;
        }
    };
    println!("Handling {} races", active_races.len());
    for race in active_races {
        handle_race(race, state, &webhooks).await;
    }
}

pub(crate) async fn cron(mut sd: Receiver<Shutdown>, webhooks: Webhooks, state: Arc<DiscordState>) {
    let tick_duration = workers::get_tick_duration(RACETIME_TICK_SECS);
    println!(
        "Starting async race worker: running every {} seconds",
        tick_duration.as_secs()
    );
    let mut intv = tokio::time::interval(tick_duration);
    loop {
        tokio::select! {
            _ = intv.tick() => {
                sweep(&state, &webhooks).await;
            }
            _sd = sd.recv() => {
                println!("async race worker shutting down");
                break;
            }
        }
    }
}
