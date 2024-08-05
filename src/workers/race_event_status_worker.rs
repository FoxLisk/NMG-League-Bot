use std::{collections::HashMap, ops::DerefMut, sync::Arc, time::Duration};

use diesel::SqliteConnection;
use itertools::Itertools as _;
use log::{info, warn};
use nmg_league_bot::{
    config::CONFIG,
    models::{
        bracket_race_infos::BracketRaceInfo,
        bracket_races::{BracketRace, BracketRaceState},
        brackets::Bracket,
        player::Player,
        season::SeasonState,
    },
    schema, NMGLeagueBotError, RaceEventError,
};
use tokio::sync::broadcast::Receiver;
use twilight_model::{
    guild::scheduled_event::{GuildScheduledEvent, Status},
    id::{marker::ScheduledEventMarker, Id},
    util::Timestamp,
};

use crate::discord::discord_state::DiscordOperations;
use crate::{
    discord::{comm_ids_and_names, discord_state::DiscordState},
    shutdown::Shutdown,
};

#[derive(Debug)]
pub struct RaceEventContent {
    pub name: String,
    pub location: String,
    /// races without commentators have no description
    pub description: Option<String>,
    /// secs from a DateTime.timestamp(), can be converted to twilight_model::util::Timestamp
    pub start: i64,
    pub end: i64,
}

impl RaceEventContent {
    fn start_timestamp(&self) -> Result<Timestamp, NMGLeagueBotError> {
        Timestamp::from_secs(self.start).map_err(From::from)
    }

    fn end_timestamp(&self) -> Result<Timestamp, NMGLeagueBotError> {
        Timestamp::from_secs(self.end).map_err(From::from)
    }
}

#[derive(Debug)]
pub enum RaceEventContentAndStatus {
    /// indicates that, if there's an event, we should complete it, and if there's not we don't create one
    Completed,
    /// indicates that we should create or update an existing event to the state defined here
    Event(RaceEventContent),
}

#[derive(Debug)]
struct RaceInfoBundle {
    race: BracketRace,
    bri: BracketRaceInfo,
    bracket: Bracket,
}

pub async fn cron(mut sd: Receiver<Shutdown>, state: Arc<DiscordState>) {
    // TODO: probably 2 minutes?
    let mut intv = tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            _sd = sd.recv() => {
                break;
            }
            _ = intv.tick() => {

                let t = tokio::time::Instant::now();
                if let Err(e) = sync_race_status(&state).await {
                    warn!("Error syncing race events: {e}");
                }
                let t2 = tokio::time::Instant::now() - t;
                info!("race event worker loop took {}ms", t2.as_millis());
            }
        }
    }
    warn!("Race event worker quit");
}

async fn sync_race_status(state: &Arc<DiscordState>) -> Result<(), NMGLeagueBotError> {
    let mut conn_o = state.diesel_cxn().await?;
    let conn = conn_o.deref_mut();
    let (race_infos, players) = get_season_race_info(conn)?;
    println!("Race infos: {race_infos:?}");
    let mut existing_events = get_existing_events_by_id(state).await?;
    for e in existing_events.values() {
        println!("Existing event: {} - status {:?}", e.name, e.status);
    }

    for mut bundle in race_infos {
        let new_status = match get_event_content(&bundle, &players, state, conn).await {
            Ok(status) => status,
            Err(e) => {
                warn!(
                    "Error getting event content for race {}: {e}",
                    bundle.race.id
                );
                continue;
            }
        };
        let existing_event = if let Some(gse_id) = bundle.bri.get_scheduled_event_id() {
            existing_events.remove(&gse_id)
        } else {
            None
        };

        match do_update_stuff(new_status, existing_event, state).await {
            Ok(Some(e)) => {
                bundle.bri.set_scheduled_event_id(e.id);
                if let Err(e) = bundle.bri.update(conn) {
                    warn!(
                        "Error updating BRI {} after creating event: {e}",
                        bundle.bri.id
                    );
                }
            }
            Ok(None) => {}
            Err(e) => {
                warn!("Error managing event for race {}: {e}", bundle.race.id);
            }
        }
    }
    Ok(())
}

async fn get_existing_events_by_id<D: DiscordOperations>(
    state: &Arc<D>,
) -> Result<HashMap<Id<ScheduledEventMarker>, GuildScheduledEvent>, NMGLeagueBotError> {
    let events = state.get_guild_scheduled_events(CONFIG.guild_id).await?;
    Ok(events
        .into_iter()
        .map(|gse| (gse.id, gse))
        .collect::<HashMap<_, _>>())
}

/// does any discord updates that are necessary (creating or updating events)
///
/// returns a GuildScheduledEvent if one is created (for persistence reasons)
// N.B. bundle parameter should probably be removed, the logging isnt really worth it
async fn do_update_stuff<D: DiscordOperations>(
    new_status: RaceEventContentAndStatus,
    existing_event: Option<GuildScheduledEvent>,
    state: &Arc<D>,
) -> Result<Option<GuildScheduledEvent>, NMGLeagueBotError> {
    match (existing_event, new_status) {
        (Some(event), RaceEventContentAndStatus::Completed) => {
            // end event (if it's not ended)
            let e = state
                .update_scheduled_event(CONFIG.guild_id, event.id)
                .status(Status::Completed)
                .await?;
            Ok(Some(e.model().await?))
        }
        (None, RaceEventContentAndStatus::Completed) => {
            // nothing to do
            Ok(None)
        }
        (Some(event), RaceEventContentAndStatus::Event(new_status)) => {
            // update the event (if necessary)
            if events_match(&event, &new_status)? {
                Ok(None)
            } else {
                state
                    .update_scheduled_event(CONFIG.guild_id, event.id)
                    .description(new_status.description.as_ref().map(|s| s.as_str()))?
                    .name(&new_status.name)?
                    .scheduled_start_time(&new_status.start_timestamp()?)
                    .scheduled_end_time(Some(&new_status.end_timestamp()?))
                    .await?;
                // it's not really interesting that an event has been updated
                Ok(None)
            }
        }
        (None, RaceEventContentAndStatus::Event(new_status)) => {
            // create
            let resp = state
                .create_scheduled_event(CONFIG.guild_id)
                .external(
                    &new_status.name,
                    &new_status.location,
                    &new_status.start_timestamp()?,
                    &new_status.end_timestamp()?,
                )?
                .await?;
            Ok(Some(resp.model().await?))
        }
    }
}

fn events_match(
    existing_event: &GuildScheduledEvent,
    new_status: &RaceEventContent,
) -> Result<bool, NMGLeagueBotError> {
    if existing_event.name == new_status.name
        && existing_event.description == new_status.description
        && existing_event.scheduled_start_time.as_secs() == new_status.start
        && existing_event.scheduled_end_time.map(|t| t.as_secs()) == Some(new_status.end)
    {
        if let Some(emd) = &existing_event.entity_metadata {
            if let Some(l) = &emd.location {
                if *l == new_status.location {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn multistream_link(p1: &Player, p2: &Player) -> String {
    format!(
        "https://multistre.am/{}/{}/layout4/",
        p1.twitch_user_login
            .clone()
            .unwrap_or("<unknown>".to_string()),
        p2.twitch_user_login
            .clone()
            .unwrap_or("<unknown>".to_string()),
    )
}

/// very unfortunate but this seems to need the discord state in order to get comm names
/// from the main discord for comms who aren't also signed up as players
async fn get_event_content<D: DiscordOperations>(
    bundle: &RaceInfoBundle,

    players: &HashMap<i32, Player>,
    state: &Arc<D>,
    conn: &mut SqliteConnection,
) -> Result<RaceEventContentAndStatus, NMGLeagueBotError> {
    if bundle.race.state()? != BracketRaceState::Scheduled {
        return Ok(RaceEventContentAndStatus::Completed);
    }
    let when = bundle
        .bri
        .scheduled()
        .ok_or(NMGLeagueBotError::MissingTimestamp)?;

    let p1 = players
        .get(&bundle.race.player_1_id)
        .ok_or(RaceEventError::MissingPlayer(bundle.race.player_1_id))?;
    let p2 = players
        .get(&bundle.race.player_2_id)
        .ok_or(RaceEventError::MissingPlayer(bundle.race.player_2_id))?;

    let event_name = format!("{}: {} vs {}", bundle.bracket.name, p1.name, p2.name);

    let event_location = match &bundle.bri.restream_channel {
        Some(s) => s.to_string(),
        None => multistream_link(p1, p2),
    };

    // NOTE: as it stands, this is going to be a behavioural change - we are going to put commentator names in
    // races that have not had commentators decided yet!
    //
    // but see https://github.com/FoxLisk/NMG-League-Bot/issues/152
    let description = {
        let comm_info = comm_ids_and_names(&bundle.bri, state, conn).await?;
        if comm_info.is_empty() {
            None
        } else {
            Some(format!(
                "with comms by {}",
                comm_info.iter().map(|(_, n)| n).join(" and ")
            ))
        }
    };

    let start = when.timestamp();
    let end = (when.clone() + chrono::Duration::minutes(100)).timestamp();

    Ok(RaceEventContentAndStatus::Event(RaceEventContent {
        name: event_name,
        location: event_location,
        description,
        start,
        end,
    }))
}

/// retrieves every race that has a BRI in the current season (i.e. every race that has been scheduled)
/// This includes completed races and races from completed rounds.
fn get_season_race_info(
    db: &mut SqliteConnection,
) -> Result<(Vec<RaceInfoBundle>, HashMap<i32, Player>), NMGLeagueBotError> {
    use diesel::prelude::*;
    let season_started_state = serde_json::to_string(&SeasonState::Started)?;
    // i think that "every race that already has a BRI and is in the current season" is the correct
    // set to look at
    // this will pick up prior round races that we'll have to look at repeatedly but i think that's not a big deal

    // N.B. if it mattered it might be worth testing if it's faster to do 1 query and pull the Bracket table every time, or
    // do a separate query for the brackets like we are doing for players
    let races = schema::bracket_race_infos::table
        .inner_join(
            schema::bracket_races::table
                .inner_join(schema::brackets::table.inner_join(schema::seasons::table)),
        )
        .select((
            BracketRace::as_select(),
            BracketRaceInfo::as_select(),
            Bracket::as_select(),
        ))
        .filter(schema::seasons::dsl::state.eq(season_started_state))
        .load(db)?
        .into_iter()
        .map(|(race, bri, bracket)| RaceInfoBundle { race, bri, bracket })
        .collect::<Vec<_>>();

    let players = Player::by_id(
        Some(
            races
                .iter()
                .map(|i| [i.race.player_1_id, i.race.player_2_id])
                .flatten()
                .collect::<_>(),
        ),
        db,
    )?;

    Ok((races, players))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use diesel::{Connection, SqliteConnection};
    use mockall::predicate::eq;
    use nmg_league_bot::{
        db::run_migrations,
        models::{
            bracket_races::{BracketRace, PlayerResult},
            brackets::{Bracket, BracketType, NewBracket},
            player::{NewPlayer, Player},
            player_bracket_entries::NewPlayerBracketEntry,
            season::{NewSeason, Season, SeasonState},
        },
        schema,
    };
    use twilight_model::id::{marker::UserMarker, Id};

    use crate::{
        discord::discord_state::MockDiscordOperations,
        workers::race_event_status_worker::{
            get_event_content, RaceEventContent, RaceEventContentAndStatus,
        },
    };

    use super::get_season_race_info;

    fn get_conn() -> anyhow::Result<SqliteConnection> {
        let mut c = SqliteConnection::establish(":memory:")?;
        run_migrations(&mut c)?;
        Ok(c)
    }

    #[allow(unused)]
    struct Fixture {
        season: Season,
        bracket: Bracket,
        players: Vec<Player>,
    }

    /// creates a season, a bracket in that season, N players in that season, and generates 1st round pairings
    fn fixtures(c: &mut SqliteConnection, nplayers: usize) -> anyhow::Result<Fixture> {
        let mut s = NewSeason::new("Any% NMG", "alttp", "Any% NMG", c)?.save(c)?;
        s.set_state(SeasonState::QualifiersOpen, c)?;
        s.set_state(SeasonState::QualifiersClosed, c)?;
        s.set_state(SeasonState::Started, c)?;
        s.update(c)?;
        let mut b = NewBracket::new(&s, "bracket", BracketType::Swiss).save(c)?;
        let mut players = Vec::with_capacity(nplayers);
        for i in 0..nplayers {
            let p = NewPlayer::new(
                format!("p{i}"),
                format!("{i}1234"),
                Some(format!("p{i}#1234")),
                Some(format!("p{i}")),
            )
            .save(c)?;
            NewPlayerBracketEntry::new(&b, &p).save(c)?;
            players.push(p);
        }

        b.generate_pairings(c)?;
        Ok(Fixture {
            season: s,
            bracket: b,
            players,
        })
    }

    #[tokio::test]
    async fn test_get_event_status_new_event() -> anyhow::Result<()> {
        let mut c = get_conn()?;
        let Fixture {
            season: _,
            bracket,
            players: _,
        } = fixtures(&mut c, 2)?;
        let (race_infos, player_map) = get_season_race_info(&mut c)?;
        // with nothing scheduled we get nothing back
        assert_eq!(0, race_infos.len());
        assert_eq!(0, player_map.len());
        let mut the_race: BracketRace = {
            use diesel::prelude::*;
            schema::bracket_races::table.first(&mut c)?
        };
        let when = Utc::now() + chrono::TimeDelta::days(1);
        the_race.schedule(&when, &mut c)?;

        let (mut race_infos, player_map) = get_season_race_info(&mut c)?;
        assert_eq!(1, race_infos.len());
        assert_eq!(2, player_map.len());
        let bundle = race_infos.pop().unwrap();
        let mock_state = MockDiscordOperations::new();
        let p1 = player_map
            .get(&bundle.race.player_1_id)
            .ok_or(anyhow::anyhow!("Missing player"))?;
        let p2 = player_map
            .get(&bundle.race.player_2_id)
            .ok_or(anyhow::anyhow!("Missing player"))?;

        let res = get_event_content(&bundle, &player_map, &Arc::new(mock_state), &mut c).await?;

        match res {
            RaceEventContentAndStatus::Completed => {
                return Err(anyhow::anyhow!("Race isn't supposed to be completed!"));
            }
            RaceEventContentAndStatus::Event(RaceEventContent {
                name,
                location,
                description,
                start,
                end,
            }) => {
                assert_eq!(
                    format!("{}: {} vs {}", bracket.name, p1.name, p2.name),
                    name
                );
                assert_eq!(
                    format!(
                        "https://multistre.am/{}/{}/layout4/",
                        p1.twitch_user_login.as_ref().unwrap(),
                        p2.twitch_user_login.as_ref().unwrap()
                    ),
                    location
                );
                assert!(description.is_none());
                assert_eq!(when.timestamp(), start);
                assert_eq!(when.timestamp() + (60 * 100), end);
            }
        };
        Ok(())
    }

    #[tokio::test]
    async fn test_get_event_status_new_event_with_comms() -> anyhow::Result<()> {
        let mut c = get_conn()?;
        let Fixture {
            season: _,
            bracket,
            players: _,
        } = fixtures(&mut c, 2)?;

        let mut the_race: BracketRace = {
            use diesel::prelude::*;
            schema::bracket_races::table.first(&mut c)?
        };
        let when = Utc::now() + chrono::TimeDelta::days(1);
        the_race.schedule(&when, &mut c)?;
        let mut info = the_race.info(&mut c)?;
        info.new_commentator_signup(Id::<UserMarker>::new(1234), &mut c)?;

        let (mut race_infos, player_map) = get_season_race_info(&mut c)?;
        assert_eq!(1, race_infos.len());
        assert_eq!(2, player_map.len());
        let bundle = race_infos.pop().unwrap();
        let mut mock_state = MockDiscordOperations::new();
        mock_state
            .expect_best_name_for()
            .with(eq(Id::<UserMarker>::new(1234)))
            .return_const("comm 1".to_string())
            .times(1);
        let p1 = player_map
            .get(&bundle.race.player_1_id)
            .ok_or(anyhow::anyhow!("Missing player"))?;
        let p2 = player_map
            .get(&bundle.race.player_2_id)
            .ok_or(anyhow::anyhow!("Missing player"))?;

        let res = get_event_content(&bundle, &player_map, &Arc::new(mock_state), &mut c).await?;

        match res {
            RaceEventContentAndStatus::Completed => {
                return Err(anyhow::anyhow!("Race isn't supposed to be completed!"));
            }
            RaceEventContentAndStatus::Event(RaceEventContent {
                name,
                location,
                description,
                start,
                end,
            }) => {
                assert_eq!(
                    format!("{}: {} vs {}", bracket.name, p1.name, p2.name),
                    name
                );
                assert_eq!(
                    format!(
                        "https://multistre.am/{}/{}/layout4/",
                        p1.twitch_user_login.as_ref().unwrap(),
                        p2.twitch_user_login.as_ref().unwrap()
                    ),
                    location
                );
                assert_eq!(Some("with comms by comm 1".to_string()), description);
                assert_eq!(when.timestamp(), start);
                assert_eq!(when.timestamp() + (60 * 100), end);
            }
        };
        Ok(())
    }

    #[tokio::test]
    async fn test_get_event_status_completed() -> anyhow::Result<()> {
        let mut c = get_conn()?;
        fixtures(&mut c, 2)?;

        let mut the_race: BracketRace = {
            use diesel::prelude::*;
            schema::bracket_races::table.first(&mut c)?
        };
        let when = Utc::now() + chrono::TimeDelta::days(-1);
        the_race.schedule(&when, &mut c)?;
        the_race.add_results(
            Some(&PlayerResult::Finish(12345)),
            Some(&PlayerResult::Forfeit),
            false,
        )?;
        the_race.update(&mut c)?;

        let (mut race_infos, player_map) = get_season_race_info(&mut c)?;
        assert_eq!(1, race_infos.len());
        assert_eq!(2, player_map.len());
        let bundle = race_infos.pop().unwrap();
        let mock_state = MockDiscordOperations::new();

        let res = get_event_content(&bundle, &player_map, &Arc::new(mock_state), &mut c).await?;

        match res {
            RaceEventContentAndStatus::Completed => Ok(()),
            RaceEventContentAndStatus::Event(rec) => Err(anyhow::anyhow!(
                "Got RaceEventContent for completed race: {rec:?}"
            )),
        }
    }

}
