use crate::constants::CRON_TICKS_VAR;
use crate::discord::discord_state::DiscordState;
use crate::schema::{bracket_race_infos, bracket_races, players};
use crate::workers::get_tick_duration;
use crate::Shutdown;
use bb8::RunError;
use chrono::{Duration, Utc};
use diesel::prelude::*;
use nmg_league_bot::models::bracket_race_infos::BracketRaceInfo;
use nmg_league_bot::models::bracket_races::{BracketRace, BracketRaceState, BracketRaceStateError, PlayerResult};
use nmg_league_bot::models::player::Player;
use racetime_api::client::RacetimeClient;
use racetime_api::endpoint::Query;
use racetime_api::endpoints::{PastCategoryRaces, PastCategoryRacesBuilder};
use racetime_api::err::RacetimeError;
use racetime_api::types::{PastRaceEntrant, RaceWithEntrants};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::ops::DerefMut;
use std::sync::Arc;
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
    PlayerResultError(#[from]  PlayerResultError),

    #[error("Error updating bracket race state: {0}")]
    BracketRaceStateError(#[from] BracketRaceStateError),
}

#[derive(Error, Debug)]
enum PlayerResultError {
    #[error("Player did not have a finish time")]
    NoFinishTime,
    #[error("Error parsing finish time")]
    ParseError(String),
}

#[derive(Deserialize, Debug)]
struct RaceStatus {
    // open
    // invitational
    // pending
    // in_progress
    // finished
    // cancelled
    value: String,
}

#[derive(Deserialize, Debug)]
struct User {
    full_name: String,
}

#[derive(Deserialize, Debug)]
struct EntrantStatus {
    // requested (requested to join)
    // invited (invited to join)
    // declined (declined invitation)
    // ready
    // not_ready
    // in_progress
    // done
    // dnf (did not finish, i.e. forfeited)
    // dq (disqualified)
    value: String,
}

#[derive(Deserialize, Debug)]
struct Entrant {
    user: User,
    status: EntrantStatus,
    finish_time: Option<String>,
    place: u32,
}

impl Entrant {
    fn result(&self) -> Result<PlayerResult, PlayerResultError> {
        match self.status.value.as_str() {
            "dnf" | "dq" => Ok(PlayerResult::Forfeit),
            "done" =>  {
                let ft = self.finish_time.as_ref().ok_or(PlayerResultError::NoFinishTime)?;
                let t= iso8601_duration::Duration::parse(ft).map_err(|e| PlayerResultError::ParseError(e.to_string()))?;
                Ok(PlayerResult::Finish(
                    t.to_std().as_secs() as u32
                ))
            }
            _ => Err(PlayerResultError::NoFinishTime)
        }
    }
}

#[derive(Deserialize, Debug)]
struct Goal {
    name: String,
}

#[derive(Deserialize, Debug)]
struct RacetimeRace {
    name: String,
    status: RaceStatus,
    url: String,
    entrants: Vec<Entrant>,
    opened_at: String,
    started_at: String,
    ended_at: String,
    goal: Goal,
}

#[derive(Deserialize, Debug)]
struct Races {
    races: Vec<RacetimeRace>,
}

/// if this `race` is one we're looking for, return all the relevant info
// TODO: check goal?
fn interesting_race<'a>(
    race: RacetimeRace,
    bracket_races: &HashMap<String, (&'a BracketRaceInfo, &'a BracketRace, &'a Player, &'a Player)>,
) -> Option<(
    &'a BracketRaceInfo,
    &'a BracketRace,
    (&'a Player, Entrant),
    (&'a Player, Entrant),
)> {
    let mut entrant_ids = race
        .entrants
        .into_iter()
        .map(|e| (e.user.full_name.clone(), e))
        .collect::<HashMap<_, _>>();

    let ids = { entrant_ids.keys().cloned().collect::<Vec<_>>() };
    for id in ids {
        // if *any* entrant is in one of the races we're looking for,
        if let Some((bri, br, p1, p2)) = bracket_races.get(&id) {
            let e1o = entrant_ids.remove(&p1.racetime_username);
            let e2o = entrant_ids.remove(&p2.racetime_username);
            if let (Some(e1), Some(e2)) = (e1o, e2o) {
                return Some((bri, br, (p1, e1), (p2, e2)));
            }
        }
    }
    None
}

async fn scan(
    state: &Arc<DiscordState>,
    racetime_client: &RacetimeClient,
) -> Result<(), ScanError> {
    let now = Utc::now();
    let start_time = now - Duration::minutes(82);
    let mut cxn = state.diesel_cxn().await?;
    let finished_state = serde_json::to_string(&BracketRaceState::Finished)?;

    let bracket_races: Vec<(BracketRaceInfo, BracketRace)> = bracket_race_infos::table
        .inner_join(bracket_races::table)
        .filter(bracket_race_infos::scheduled_for.lt(start_time.timestamp()))
        .filter(bracket_races::state.ne(finished_state))
        .load(cxn.deref_mut())?;

    // *shrug*
    // it's like 40 rows
    let all_players: Vec<Player> = players::table.load(cxn.deref_mut())?;
    let players_lookup: HashMap<i32, Player> = all_players.into_iter().map(|p| (p.id, p)).collect();

    let mut interesting_rtgg_ids: HashMap<
        String,
        (&BracketRaceInfo, &BracketRace, &Player, &Player),
    > = Default::default();

    for (bri, br) in &bracket_races {
        let p1 = match players_lookup.get(&br.player_1_id) {
            Some(p) => p,
            None => {
                continue;
            }
        };
        let p2 = match players_lookup.get(&br.player_2_id) {
            Some(p) => p,
            None => {
                continue;
            }
        };
        interesting_rtgg_ids.insert(p1.racetime_username.clone(), (bri, br, p1, p2));
    }

    let recent_races: PastCategoryRaces = PastCategoryRacesBuilder::default()
        .show_entrants(true)
        .category("alttp")
        .build()?;

    let finished_races: Races = recent_races.query(racetime_client).await?;

    for race in finished_races.races {
        if let Some((bri, br, p1, p2)) = interesting_race(
            race, &interesting_rtgg_ids
        ) {
            // this is awful, i hate doing it this way, i'm just tired of thinking about this
            let mut mutable_br = br.clone();
            if let Err(e) = update_race_stuff(mutable_br, p1, p2, cxn.deref_mut()) {
                println!("Error updating {:?}: {}", br, e);
            }
        }
    }
    Ok(())
}



fn update_race_stuff(mut br: BracketRace, p1_info: (&Player, Entrant), p2_info: (&Player, Entrant), conn: &mut SqliteConnection) -> Result<(), ScanError> {
    let (_p1, e1) = p1_info;
    let (_p2, e2) = p2_info;
    let p1_result = e1.result()?;
    let p2_result = e2.result()?;
    br.add_results(Some(p1_result), Some(p2_result))?;
    br.update(conn)?;
    Ok(())
}


pub async fn cron(mut sd: Receiver<Shutdown>, state: Arc<DiscordState>) {
    let tick_duration = get_tick_duration(CRON_TICKS_VAR);
    println!(
        "Starting racetime scanner worker: running every {} seconds",
        tick_duration.as_secs()
    );
    let mut intv = tokio::time::interval(tick_duration);
    let client = RacetimeClient::new().unwrap();
    loop {
        tokio::select! {
            _ = intv.tick() => {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use crate::workers::racetime_scanner_worker::{Entrant, EntrantStatus, Goal, interesting_race, RaceStatus, RacetimeRace, User};
    use iso8601_duration::Duration;
    use racetime_api::types::{Race, RaceWithEntrants};
    use std::fmt::Debug;
    use nmg_league_bot::models::bracket_race_infos::BracketRaceInfo;
    use nmg_league_bot::models::bracket_races::BracketRace;
    use nmg_league_bot::models::player::Player;

    #[test]
    fn test_interesting_race() {
        let race = RacetimeRace {
            name: "asdf".to_string(),
            status: RaceStatus {
                value: "finished".to_string(),
            },
            url: "asdf".to_string(),
            entrants: vec![
                Entrant {
                    user: User {
                        full_name: "p1#1234".to_string(),
                    },
                    status: EntrantStatus {
                        value: "done".to_string(),
                    },
                    finish_time: Option::from("PT1H23M45S".to_string()),
                    place: 1,
                },
                Entrant {
                    user: User {
                        full_name: "p2#1234".to_string(),
                    },
                    status: EntrantStatus {
                        value: "done".to_string(),
                    },
                    finish_time: Option::from("PT1H34M56S".to_string()),
                    place: 2,
                },
            ],
            opened_at: "".to_string(),
            started_at: "".to_string(),
            ended_at: "".to_string(),
            goal: Goal {
                name: "Any% NMG".to_string()
            }
        };
        let br = BracketRace {
            id: 1,
            bracket_id: 1,
            round_id: 1,
            player_1_id: 1,
            player_2_id: 2,
            async_race_id: None,
            state: "Scheduled".to_string(),
            player_1_result: None,
            player_2_result: None,
            outcome: None
        };
        let bri = BracketRaceInfo {
            id: 1,
            bracket_race_id: 1,
            scheduled_for: None, // little white lie
            scheduled_event_id: None,
            commportunities_message_id: None,
            restream_request_message_id: None
        };
        let p1 = Player{
            id: 1,
            name: "player 1".to_string(),
            discord_id: "1234".to_string(),
            racetime_username: "p1#1234".to_string(),
            restreams_ok: 1
        };
        let p2 = Player{
            id: 2,
            name: "player 2".to_string(),
            discord_id: "3456".to_string(),
            racetime_username: "p2#1234".to_string(),
            restreams_ok: 1
        };
        let mut races = HashMap::new();
        races.insert(p1.racetime_username.clone(), (&bri, &br, &p1, &p2));
        races.insert(p2.racetime_username.clone(), (&bri, &br, &p1, &p2));
        let whatever = interesting_race(race, &races);
        assert!(whatever.is_some());
        let (_, _, (p1, e1), (p2, e2)) = whatever.unwrap();
        assert_eq!(p1.racetime_username, e1.user.full_name);
        assert_eq!(p2.racetime_username, e2.user.full_name);
    }


    #[test]
    fn test_uninteresting_race() {
        let race = RacetimeRace {
            name: "asdf".to_string(),
            status: RaceStatus {
                value: "finished".to_string(),
            },
            url: "asdf".to_string(),
            entrants: vec![
                Entrant {
                    user: User {
                        full_name: "p1#1234".to_string(),
                    },
                    status: EntrantStatus {
                        value: "done".to_string(),
                    },
                    finish_time: Option::from("PT1H23M45S".to_string()),
                    place: 1,
                },
                Entrant {
                    user: User {
                        full_name: "p2#1234".to_string(),
                    },
                    status: EntrantStatus {
                        value: "done".to_string(),
                    },
                    finish_time: Option::from("PT1H34M56S".to_string()),
                    place: 2,
                },
            ],
            opened_at: "".to_string(),
            started_at: "".to_string(),
            ended_at: "".to_string(),

            goal: Goal {
                name: "Any% NMG".to_string()
            }
        };
        let br = BracketRace {
            id: 1,
            bracket_id: 1,
            round_id: 1,
            player_1_id: 1,
            player_2_id: 2,
            async_race_id: None,
            state: "Scheduled".to_string(),
            player_1_result: None,
            player_2_result: None,
            outcome: None
        };
        let bri = BracketRaceInfo {
            id: 1,
            bracket_race_id: 1,
            scheduled_for: None, // little white lie
            scheduled_event_id: None,
            commportunities_message_id: None,
            restream_request_message_id: None
        };
        let p1 = Player{
            id: 1,
            name: "player 1".to_string(),
            discord_id: "1234".to_string(),
            racetime_username: "p1#1234".to_string(),
            restreams_ok: 1
        };
        let p3 = Player{
            id: 3,
            name: "player 3".to_string(),
            discord_id: "3456".to_string(),
            racetime_username: "p3#1234".to_string(),
            restreams_ok: 1
        };
        let mut races = HashMap::new();
        races.insert(p1.racetime_username.clone(), (&bri, &br, &p1, &p3));
        races.insert(p3.racetime_username.clone(), (&bri, &br, &p1, &p3));
        let whatever = interesting_race(race, &races);
        assert!(whatever.is_none());
    }
}
