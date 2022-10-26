/*
this shitty module is stuff for workers::* to call so that I can also call it from test code
 */

use crate::models::bracket_race_infos::BracketRaceInfo;
use crate::models::bracket_races::{
    BracketRace, BracketRaceState, BracketRaceStateError, Outcome, PlayerResult,
};
use crate::models::player::Player;
use crate::racetime_types::{Entrant, RacetimeRace};
use crate::schema::{bracket_race_infos, bracket_races};
use crate::ChannelConfig;
use chrono::{Duration, Utc};
use diesel::prelude::*;
use diesel::SqliteConnection;
use std::collections::HashMap;
use thiserror::Error;
use twilight_http::Client;
use twilight_model::channel::embed::{Embed, EmbedField};
use twilight_validate::message::MessageValidationError;

/// races that are not in finished state and that are scheduled to have started recently
pub fn get_races_that_should_be_finishing_soon(
    conn: &mut SqliteConnection,
) -> Result<Vec<(BracketRaceInfo, BracketRace)>, diesel::result::Error> {
    let now = Utc::now();
    let start_time = now - Duration::minutes(82);
    // TODO: pretend to care about this unwrap later maybe
    let finished_state = serde_json::to_string(&BracketRaceState::Finished).unwrap();

    bracket_race_infos::table
        .inner_join(bracket_races::table)
        .filter(bracket_race_infos::scheduled_for.lt(start_time.timestamp()))
        .filter(bracket_races::state.ne(finished_state))
        .load(conn)
}

pub fn races_by_player_rtgg<'a>(
    all_players: &'a [Player],
    bracket_races: &'a [(BracketRaceInfo, BracketRace)],
) -> HashMap<String, (&'a BracketRaceInfo, &'a BracketRace, &'a Player, &'a Player)> {
    let players_lookup: HashMap<i32, &Player> = all_players.iter().map(|p| (p.id, p)).collect();

    let mut interesting_rtgg_ids: HashMap<
        String,
        (&BracketRaceInfo, &BracketRace, &Player, &Player),
    > = Default::default();

    for (bri, br) in bracket_races {
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
    interesting_rtgg_ids
}

/// if this `race` is one we're looking for, return all the relevant info
/// consumes race.entrants
pub fn interesting_race<'a>(
    race: &mut RacetimeRace,
    bracket_races: &HashMap<String, (&'a BracketRaceInfo, &'a BracketRace, &'a Player, &'a Player)>,
) -> Option<(
    &'a BracketRaceInfo,
    &'a BracketRace,
    (&'a Player, Entrant),
    (&'a Player, Entrant),
)> {
    if race.goal.name != "Any% NMG" {
        return None;
    }
    if race.status.value != "finished" {
        return None;
    }
    let mut entrant_ids = std::mem::take(&mut race.entrants)
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

#[derive(Debug, Error)]
pub enum RaceFinishError {
    #[error("Bracket race state error: {0}")]
    BracketRaceStateError(#[from] BracketRaceStateError),

    #[error("Deserialization error: {0}")]
    DeserializationError(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    DatabaseError(#[from] diesel::result::Error),

    #[error("MessageValidationError: {0}")]
    MessageValidationError(#[from] MessageValidationError),

    #[error("HTTP error: {0}")]
    HttpError(#[from] twilight_http::Error),

    #[error("Race wasn't finished?")]
    NotFinished,
}

/**
This function does these things:

    1. set the result fields on the race, and update its state to finished if relevant
    2. saves that race
    3. if a [Client] is supplied, posts a message in #match-results
*/
pub async fn trigger_race_finish(
    mut br: BracketRace,
    bri: &BracketRaceInfo,
    p1: (&Player, PlayerResult),
    p2: (&Player, PlayerResult),
    conn: &mut SqliteConnection,
    client: Option<&Client>,
    channel_config: &ChannelConfig,
) -> Result<(), RaceFinishError> {
    let (p1, p1res) = p1;
    let (p2, p2res) = p2;
    br.add_results(Some(&p1res), Some(&p2res))?;
    br.update(conn)?;

    if let Some(c) = client {
        let bracket = br.bracket(conn)?;
        let outcome = br.outcome()?.ok_or(RaceFinishError::NotFinished)?;
        let winner = match outcome {
            Outcome::Tie => {
                format!(
                    "It's a tie?! **{}** ({}) vs **{}** ({})",
                    p1.name, p1res, p2.name, p2res
                )
            }
            Outcome::P1Win => {
                format!(
                    "**{}** ({}) defeats **{}** ({})",
                    p1.name, p1res, p2.name, p2res
                )
            }
            Outcome::P2Win => {
                format!(
                    "**{}** ({}) defeats **{}** ({})",
                    p2.name, p2res, p1.name, p1res
                )
            }
        };
        let mut fields = vec![EmbedField {
            inline: false,
            name: "Division".to_string(),
            value: bracket.name,
        }];
        if let Some(url) = &bri.racetime_gg_url {
            fields.push(EmbedField {
                inline: false,
                name: "RaceTime room".to_string(),
                value: url.clone(),
            })
        }

        let embed = Embed {
            author: None,
            color: None,
            description: Some(winner),
            fields,
            footer: None,
            image: None,
            kind: "rich".to_string(),
            provider: None,
            thumbnail: None,
            timestamp: None,
            title: None,
            url: None,
            video: None,
        };
        c
            .create_message(channel_config.match_results)
            .embeds(&vec![embed])?
            .exec()
            .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::models::bracket_race_infos::BracketRaceInfo;
    use crate::models::bracket_races::BracketRace;
    use crate::models::player::Player;
    use crate::racetime_types::{
        Entrant, EntrantStatus, Goal, RaceStatus, Races, RacetimeRace, User,
    };
    use crate::worker_funcs::interesting_race;
    use std::collections::HashMap;
    use std::fs::read_to_string;

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
                },
                Entrant {
                    user: User {
                        full_name: "p2#1234".to_string(),
                    },
                    status: EntrantStatus {
                        value: "done".to_string(),
                    },
                    finish_time: Option::from("PT1H34M56S".to_string()),
                },
            ],
            opened_at: "".to_string(),
            started_at: "".to_string(),
            ended_at: "".to_string(),
            goal: Goal {
                name: "Any% NMG".to_string(),
            },
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
            outcome: None,
        };
        let bri = BracketRaceInfo {
            id: 1,
            bracket_race_id: 1,
            scheduled_for: None, // little white lie
            scheduled_event_id: None,
            commportunities_message_id: None,
            restream_request_message_id: None,
        };
        let p1 = Player {
            id: 1,
            name: "player 1".to_string(),
            discord_id: "1234".to_string(),
            racetime_username: "p1#1234".to_string(),
            restreams_ok: 1,
        };
        let p2 = Player {
            id: 2,
            name: "player 2".to_string(),
            discord_id: "3456".to_string(),
            racetime_username: "p2#1234".to_string(),
            restreams_ok: 1,
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
                },
                Entrant {
                    user: User {
                        full_name: "p2#1234".to_string(),
                    },
                    status: EntrantStatus {
                        value: "done".to_string(),
                    },
                    finish_time: Option::from("PT1H34M56S".to_string()),
                },
            ],
            opened_at: "".to_string(),
            started_at: "".to_string(),
            ended_at: "".to_string(),

            goal: Goal {
                name: "Any% NMG".to_string(),
            },
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
            outcome: None,
        };
        let bri = BracketRaceInfo {
            id: 1,
            bracket_race_id: 1,
            scheduled_for: None, // little white lie
            scheduled_event_id: None,
            commportunities_message_id: None,
            restream_request_message_id: None,
        };
        let p1 = Player {
            id: 1,
            name: "player 1".to_string(),
            discord_id: "1234".to_string(),
            racetime_username: "p1#1234".to_string(),
            restreams_ok: 1,
        };
        let p3 = Player {
            id: 3,
            name: "player 3".to_string(),
            discord_id: "3456".to_string(),
            racetime_username: "p3#1234".to_string(),
            restreams_ok: 1,
        };
        let mut races = HashMap::new();
        races.insert(p1.racetime_username.clone(), (&bri, &br, &p1, &p3));
        races.insert(p3.racetime_username.clone(), (&bri, &br, &p1, &p3));
        let whatever = interesting_race(race, &races);
        assert!(whatever.is_none());
    }

    #[test]
    fn test_deserialize() {
        let contents = read_to_string("test_data/recent_alttp_races.json").unwrap();

        let value: serde_json::Value = serde_json::from_str(&contents).unwrap();
        let races = value.get("races").unwrap();
        for race in races.as_array().unwrap() {
            let re_serialized = serde_json::to_string(race).unwrap();
            let re_de_ser_ial_ized = serde_json::from_str::<RacetimeRace>(&re_serialized);
            assert!(
                re_de_ser_ial_ized.is_ok(),
                "{}: {:?}",
                re_serialized,
                re_de_ser_ial_ized
            );
        }

        let races = serde_json::from_str::<Races>(&contents);
        assert!(races.is_ok());
    }
}
