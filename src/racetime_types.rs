use chrono::{DateTime, FixedOffset};
use serde::Deserialize;
use thiserror::Error;
use crate::models::bracket_races::PlayerResult;

#[derive(Error, Debug)]
pub enum PlayerResultError {
    #[error("Player did not have a finish time")]
    NoFinishTime,
    #[error("Error parsing finish time")]
    ParseError(String),
}

#[derive(Deserialize, Debug)]
pub struct RaceStatus {
    // open
    // invitational
    // pending
    // in_progress
    // finished
    // cancelled
    pub   value: String,
}

#[derive(Deserialize, Debug)]
pub struct User {
    pub  full_name: String,
}

#[derive(Deserialize, Debug)]
pub struct EntrantStatus {
    // requested (requested to join)
    // invited (invited to join)
    // declined (declined invitation)
    // ready
    // not_ready
    // in_progress
    // done
    // dnf (did not finish, i.e. forfeited)
    // dq (disqualified)
    pub value: String,
}

#[derive(Deserialize, Debug)]
pub struct Entrant {
    pub user: User,
    pub status: EntrantStatus,
    pub finish_time: Option<String>,
}

impl Entrant {
    pub fn result(&self) -> Result<PlayerResult, PlayerResultError> {
        match self.status.value.as_str() {
            "dnf" | "dq" => Ok(PlayerResult::Forfeit),
            "done" => {
                let ft = self
                    .finish_time
                    .as_ref()
                    .ok_or(PlayerResultError::NoFinishTime)?;
                let t = iso8601_duration::Duration::parse(ft)
                    .map_err(|e| PlayerResultError::ParseError(e.to_string()))?;
                Ok(PlayerResult::Finish(t.to_std().as_secs() as u32))
            }
            _ => Err(PlayerResultError::NoFinishTime),
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct Goal {
    pub name: String,
}

#[derive(Deserialize, Debug)]
#[allow(unused)]
pub struct RacetimeRace {
    pub name: String,
    pub status: RaceStatus,
    pub url: String,
    pub entrants: Vec<Entrant>,
    pub opened_at: String,
    pub started_at: String,
    pub ended_at: String,
    pub goal: Goal,
}

impl RacetimeRace {
    pub fn started_at(&self) -> Result<DateTime<FixedOffset>, chrono::ParseError> {
        DateTime::parse_from_rfc3339(&self.started_at)
    }
}

#[derive(Deserialize, Debug)]
pub struct Races {
    pub races: Vec<RacetimeRace>,
}

#[cfg(test)]
mod tests {
    use chrono::{Datelike, DateTime, Timelike};

    #[test]
    fn test_parse_rtgg_date() {
        let date = "2022-10-23T18:45:05.135Z";
        let dtr = DateTime::parse_from_rfc3339(date);
        assert!(dtr.is_ok());
        let dt = dtr.unwrap();
        assert_eq!(2022, dt.year());
        assert_eq!(10, dt.month());
        assert_eq!(23, dt.day());
        assert_eq!(18, dt.hour());

    }
}