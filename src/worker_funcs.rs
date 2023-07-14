/*
this shitty module is stuff for workers::* to call so that I can also call it from test code
 */

use crate::models::bracket_race_infos::BracketRaceInfo;
use crate::models::bracket_races::{BracketRace, BracketRaceStateError, Outcome, PlayerResult};
use crate::models::player::Player;
use crate::models::season::Season;
use crate::racetime_types::{Entrant, RacetimeRace};
use crate::{ChannelConfig, NMGLeagueBotError};
use diesel::SqliteConnection;
use log::{debug, info, warn};
use std::collections::HashMap;
use thiserror::Error;
use twilight_http::response::DeserializeBodyError;
use twilight_http::Client;
use twilight_model::channel::message::embed::{Embed, EmbedField};
use twilight_model::channel::Message;
use twilight_model::id::marker::ChannelMarker;
use twilight_model::id::Id;
use twilight_validate::message::MessageValidationError;

/// takes a list of all existing players & bracket races, and returns a map of
/// <one of the player's racetime usernames : a bunch of info about the race>
///
/// this is sort of insane, right?
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
        match (&p1.racetime_username, &p2.racetime_username) {
            (Some(rtu), Some(_)) => {
                interesting_rtgg_ids.insert(rtu.clone(), (bri, br, p1, p2));
            }
            _ => {
                // we need to know both players' rtgg usernames to find out that a race contains
                // both of them
            }
        };
    }
    interesting_rtgg_ids
}

/// if this `race` is one we're looking for, return all the relevant info
/// consumes race.entrants
pub fn interesting_race<'a>(
    race: &mut RacetimeRace,
    bracket_races: &HashMap<String, (&'a BracketRaceInfo, &'a BracketRace, &'a Player, &'a Player)>,
    season: &Season,
) -> Option<(
    &'a BracketRaceInfo,
    &'a BracketRace,
    (&'a Player, Entrant),
    (&'a Player, Entrant),
)> {
    if &race.goal.name != &season.rtgg_goal_name {
        debug!("Skipping because invalid goal name {}", race.goal.name);
        return None;
    }
    if race.status.value != "finished" {
        debug!(
            "Skipping because race isn't finished yet (status {})",
            race.status.value
        );
        return None;
    }
    let started = match race.started_at() {
        Ok(dt) => dt,
        Err(e) => {
            warn!(
                "Error parsing racetime started_at ({}): {e}",
                race.started_at
            );
            return None;
        }
    };

    let mut entrant_ids = std::mem::take(&mut race.entrants)
        .into_iter()
        .map(|e| (e.user.full_name.to_lowercase(), e))
        .collect::<HashMap<_, _>>();

    let ids = { entrant_ids.keys().cloned().collect::<Vec<_>>() };
    for id in ids {
        // if *any* entrant is in one of the races we're looking for, let's check if they all are
        if let Some((bri, br, p1, p2)) = bracket_races.get(&id) {
            debug!("Found interesting rtgg id {id}, looking closer");
            // but, okay, let's not pick up a weekly from 2 months ago, lmao
            let scheduled = match bri.scheduled() {
                Some(dt) => dt,
                None => {
                    warn!("Checking if a racetime race is interesting but the bracket race wasn't scheduled? {:?}", bri);
                    continue;
                }
            };
            if scheduled.signed_duration_since(started).num_minutes() > 180 {
                info!(
                    "This race ({}) was started a very long time ago: {}",
                    race.name, race.started_at
                );
                continue;
            }
            let p1rt = match &p1.racetime_username {
                Some(s) => s,
                None => {
                    continue;
                }
            };
            let p2rt = match &p2.racetime_username {
                Some(s) => s,
                None => {
                    continue;
                }
            };
            debug!("Found rt usernames for both players: {p1rt} vs {p2rt}");

            let e1o = entrant_ids.remove(p1rt);
            let e2o = entrant_ids.remove(p2rt);
            debug!("Found these entrants: {e1o:?}, {e2o:?}");
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

    #[error("Error deserializing discord response: {0}")]
    DeserializeBodyError(#[from] DeserializeBodyError),
}

// this really really sucks. lots of this stuff should be references, but that's just
// not super how models work, or at least not the way they are working here in this application
// so this ends up being constructed like "hurr durr race.clone()" sad sad sad
pub struct RaceFinishOptions {
    pub bracket_race: BracketRace,
    pub info: BracketRaceInfo,
    pub player_1: Player,
    pub player_1_result: PlayerResult,
    pub player_2: Player,
    pub player_2_result: PlayerResult,
    pub channel_id: Id<ChannelMarker>,
    pub force_update: bool,
}

/**
This function does these things:

1. set the result fields on the race, and update its state to finished if relevant
2. saves that race
3. if a [Client] is supplied, posts a message in #match-results
*/
pub async fn trigger_race_finish(
    mut options: RaceFinishOptions,
    conn: &mut SqliteConnection,
    client: Option<&Client>,
    channel_config: &ChannelConfig,
) -> Result<(), RaceFinishError> {
    options.bracket_race.add_results(
        Some(&options.player_1_result),
        Some(&options.player_2_result),
        options.force_update,
    )?;
    options.bracket_race.update(conn)?;

    if let Some(c) = client {
        if let Err(e) = post_match_results(c, &options, conn).await {
            warn!(
                "Error posting match results for race {}: {e}",
                options.bracket_race.id
            );
        }
        if let Err(e) = clear_commportunities_message(&mut options.info, c, channel_config).await {
            warn!(
                "Error clearing commportunities message for race {}: {e}",
                options.bracket_race.id
            );
        }
        // TODO: maybe clear other messages? under some circumstances?
    }

    Ok(())
}

async fn post_match_results(
    c: &Client,
    options: &RaceFinishOptions,
    conn: &mut SqliteConnection,
) -> Result<Message, RaceFinishError> {
    let bracket = options.bracket_race.bracket(conn)?;
    let outcome = options
        .bracket_race
        .outcome()?
        .ok_or(RaceFinishError::NotFinished)?;
    let winner = match outcome {
        Outcome::Tie => {
            format!(
                "It's a tie?! **{}** ({}) vs **{}** ({})",
                options.player_1.name,
                options.player_1_result,
                options.player_2.name,
                options.player_2_result
            )
        }
        Outcome::P1Win => {
            format!(
                "**{}** ({}) defeats **{}** ({})",
                options.player_1.name,
                options.player_1_result,
                options.player_2.name,
                options.player_2_result
            )
        }
        Outcome::P2Win => {
            format!(
                "**{}** ({}) defeats **{}** ({})",
                options.player_2.name,
                options.player_2_result,
                options.player_1.name,
                options.player_1_result
            )
        }
    };
    let mut fields = vec![EmbedField {
        inline: false,
        name: "Division".to_string(),
        value: bracket.name,
    }];
    if let Some(url) = &options.info.racetime_gg_url {
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
    let m = c
        .create_message(options.channel_id)
        .embeds(&vec![embed])?
        .await?;
    m.model().await.map_err(From::from)
}

macro_rules! clear_bracket_race_info_message {
    ($info:expr, $client:expr, $channel_id:expr, $mid_fn:ident, $clear_fn:ident) => {
        if let Some(mid) = $info.$mid_fn() {
            match $client.delete_message($channel_id, mid).await {
                Ok(_) => {
                    $info.$clear_fn();
                    Ok(true)
                }
                Err(e) => match e.kind() {
                    twilight_http::error::ErrorType::Response { status, .. } => {
                        if status.get() == 404 {
                            $info.$clear_fn();
                            Ok(true)
                        } else {
                            Err(From::from(e))
                        }
                    }
                    _ => Err(From::from(e)),
                },
            }
        } else {
            Ok(false)
        }
    };
}

/// deletes the message (if any) and if the delete is successful, nulls the model field
/// does not persist `info`
/// Returns true if dirty
pub async fn clear_tentative_commentary_assignment_message(
    info: &mut BracketRaceInfo,
    client: &Client,
    channel_config: &ChannelConfig,
) -> Result<bool, NMGLeagueBotError> {
    clear_bracket_race_info_message!(
        info,
        client,
        channel_config.commentary_discussion,
        get_tentative_commentary_assignment_message_id,
        clear_tentative_commentary_assignment_message_id
    )
}

/// deletes the message (if any) and if the delete is successful, nulls the model field
/// does not persist `info`
/// Returns true if dirty
pub async fn clear_commportunities_message(
    info: &mut BracketRaceInfo,
    client: &Client,
    channel_config: &ChannelConfig,
) -> Result<bool, NMGLeagueBotError> {
    clear_bracket_race_info_message!(
        info,
        client,
        channel_config.commportunities,
        get_commportunities_message_id,
        clear_commportunities_message_id
    )
}

#[cfg(test)]
mod tests {
    use crate::models::bracket_race_infos::BracketRaceInfo;
    use crate::models::bracket_races::BracketRace;
    use crate::models::player::Player;
    use crate::models::season::Season;
    use crate::racetime_types::{
        Entrant, EntrantStatus, Goal, RaceStatus, Races, RacetimeRace, User,
    };
    use crate::worker_funcs::interesting_race;
    use chrono::{DateTime, TimeZone, Utc};
    use std::collections::HashMap;
    use std::fs::read_to_string;

    #[test]
    fn test_rfc3339() {
        let whatever = DateTime::parse_from_rfc3339("2022-11-02T00:58:02.790200800+00:00");
        assert!(whatever.is_ok());
    }
    #[test]
    fn test_date_math() {
        let scheduled = Utc.timestamp(1667349918, 0);
        let race_started_very_old =
            DateTime::parse_from_rfc3339("2022-10-23T19:07:20.025Z").unwrap();
        let dur = scheduled.signed_duration_since(race_started_very_old);
        assert!(dur.num_minutes() > 100);
    }

    fn bracket_race_info(
        id: i32,
        bracket_race_id: i32,
        when: Option<Option<DateTime<Utc>>>,
    ) -> BracketRaceInfo {
        let scheduled_for = match when {
            Some(Some(dt)) => Some(dt.timestamp()),
            Some(None) => Some(Utc::now().timestamp()),
            None => None,
        };
        BracketRaceInfo {
            id,
            bracket_race_id,
            scheduled_for,
            scheduled_event_id: None,
            commportunities_message_id: None,
            restream_request_message_id: None,
            racetime_gg_url: None,
            tentative_commentary_assignment_message_id: None,
            commentary_assignment_message_id: None,
            restream_channel: None,
        }
    }

    #[test]
    fn test_interesting_race() {
        let now = Utc::now();
        let mut race = RacetimeRace {
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
            started_at: now.to_rfc3339(),
            ended_at: "".to_string(),
            goal: Goal {
                name: "Any% NMG".to_string(),
            },
        };
        let season = Season::new(1, "Any% NMG");
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
        let bri = bracket_race_info(1, 1, Some(None));
        let p1 = Player {
            id: 1,
            name: "player 1".to_string(),
            discord_id: "1234".to_string(),
            racetime_username: Some("p1#1234".to_string()),
            twitch_user_login: Some("p1_ttv".to_string()),
            restreams_ok: 1,
        };
        let p2 = Player {
            id: 2,
            name: "player 2".to_string(),
            discord_id: "3456".to_string(),
            racetime_username: Some("p2#1234".to_string()),
            twitch_user_login: Some("p2_ttv".to_string()),
            restreams_ok: 1,
        };
        let mut races = HashMap::new();
        races.insert(p1.racetime_username.clone().unwrap(), (&bri, &br, &p1, &p2));
        races.insert(p2.racetime_username.clone().unwrap(), (&bri, &br, &p1, &p2));
        let whatever = interesting_race(&mut race, &races, &season);
        assert!(whatever.is_some(), "{:?}", whatever);
        let (_, _, (p1, e1), (p2, e2)) = whatever.unwrap();
        assert_eq!(p1.racetime_username, Some(e1.user.full_name));
        assert_eq!(p2.racetime_username, Some(e2.user.full_name));
    }

    #[test]
    fn test_uninteresting_race() {
        let now = Utc::now();
        let mut race = RacetimeRace {
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
            started_at: now.to_rfc3339(),
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
        let bri = bracket_race_info(1, 1, Some(None));
        let p1 = Player {
            id: 1,
            name: "player 1".to_string(),
            discord_id: "1234".to_string(),
            racetime_username: Option::from("p1#1234".to_string()),
            restreams_ok: 1,
            twitch_user_login: Option::from("p1_ttv".to_string()),
        };
        let p3 = Player {
            id: 3,
            name: "player 3".to_string(),
            discord_id: "3456".to_string(),
            racetime_username: Option::from("p3#1234".to_string()),
            twitch_user_login: Option::from("p3_ttv".to_string()),
            restreams_ok: 1,
        };
        let mut races = HashMap::new();
        races.insert(
            p1.racetime_username.clone().unwrap().to_lowercase(),
            (&bri, &br, &p1, &p3),
        );
        races.insert(
            p3.racetime_username.clone().unwrap().to_lowercase(),
            (&bri, &br, &p1, &p3),
        );
        let season = Season::new(1, "Any% NMG");
        let whatever = interesting_race(&mut race, &races, &season);
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
