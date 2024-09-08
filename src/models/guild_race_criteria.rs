use std::collections::HashMap;

use crate::{
    delete_fn, save_fn,
    schema::guild_race_criteria::{self, guild_id},
    NMGLeagueBotError,
};
use diesel::prelude::*;
use log::warn;
use serde::Serialize;
use twilight_model::id::{marker::GuildMarker, Id};

use super::{bracket_race_infos::BracketRaceInfo, bracket_races::BracketRace, player::Player};

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub enum RestreamStatusCriterion {
    HasRestream,
    HasNoRestream,
    Any,
}

#[derive(Queryable, Identifiable, Debug, AsChangeset, Serialize, Clone, Selectable)]
#[diesel(treat_none_as_null = true)]
#[diesel(table_name = guild_race_criteria)]
pub struct GuildRaceCriteria {
    pub id: i32,
    guild_id: String,
    pub player_id: Option<i32>,
    // i usually dont love tristate variables but it seems appropriate here?
    // either you care, and then its yes or no, or you don't care.
    /// true means restream required, false means restream forbidden
    restream_status: Option<bool>,
}

impl GuildRaceCriteria {
    pub fn get_by_id(
        id: i32,
        gid: Id<GuildMarker>,
        conn: &mut SqliteConnection,
    ) -> Result<Option<Self>, diesel::result::Error> {
        guild_race_criteria::table
            .filter(guild_race_criteria::dsl::id.eq(id))
            .filter(guild_race_criteria::dsl::guild_id.eq(gid.to_string()))
            .first(conn)
            .optional()
    }

    pub fn list_for_guild(
        gid: Id<GuildMarker>,
        conn: &mut SqliteConnection,
    ) -> Result<Vec<Self>, diesel::result::Error> {
        guild_race_criteria::table
            .filter(guild_race_criteria::dsl::guild_id.eq(gid.to_string()))
            .load(conn)
    }

    fn race_is_interesting(&self, race: &BracketRace, bri: &BracketRaceInfo) -> bool {
        let pass_player = if let Some(pid) = self.player_id {
            race.player_1_id == pid || race.player_2_id == pid
        } else {
            true
        };
        let pass_restream = if let Some(st) = self.restream_status {
            st == bri.restream_channel.is_some()
        } else {
            true
        };
        pass_player && pass_restream
    }

    // if you pass in the wrong Player, you'll get a String back that uses "<unknown player>" for their name
    pub fn display(&self, player: Option<&Player>) -> String {
        let player_str = match (self.player_id, player) {
            (Some(pid), Some(p)) => {
                if p.id == pid {
                    p.name.as_str()
                } else {
                    "<unknown player>"
                }
            }
            (Some(_), None) => "<unknown player>",
            (None, _) => "any player",
        };
        let restream_str = match self.restream_status {
            Some(true) => "restream",
            Some(false) => "no restream",
            None => "or without restream",
        };
        format!("Races featuring {player_str}, with {restream_str}")
    }

    delete_fn!(crate::schema::guild_race_criteria::table);
}

#[derive(Debug)]
pub struct GuildCriteria {
    guild_id: Id<GuildMarker>,
    criteria: Vec<GuildRaceCriteria>,
}

impl GuildCriteria {
    pub fn guild_id(&self) -> Id<GuildMarker> {
        self.guild_id
    }

    pub fn race_is_interesting(&self, race: &BracketRace, bri: &BracketRaceInfo) -> bool {
        self.criteria
            .iter()
            .any(|f| f.race_is_interesting(race, bri))
    }
}

#[derive(Insertable)]
#[diesel(table_name=guild_race_criteria)]
pub struct NewGuildRaceCriteria {
    guild_id: String,
    player_id: Option<i32>,
    restream_status: Option<bool>,
}

impl NewGuildRaceCriteria {
    pub fn new(
        gid: Id<GuildMarker>,
        player: Option<Player>,
        restream_status: RestreamStatusCriterion,
    ) -> Self {
        Self {
            guild_id: gid.to_string(),
            player_id: player.map(|p| p.id),
            restream_status: match restream_status {
                RestreamStatusCriterion::HasRestream => Some(true),
                RestreamStatusCriterion::HasNoRestream => Some(false),
                RestreamStatusCriterion::Any => None,
            },
        }
    }
    save_fn!(guild_race_criteria::table, GuildRaceCriteria);
}

/// Gets all criteria mapped by guild_id.
pub fn race_criteria_by_guild_id<'a, I: Iterator<Item = &'a Id<GuildMarker>>>(
    ids: I,
    conn: &mut SqliteConnection,
) -> Result<HashMap<Id<GuildMarker>, GuildCriteria>, NMGLeagueBotError> {
    let guild_ids = ids.collect::<Vec<_>>();
    let raw_criteria = guild_race_criteria::table
        .filter(guild_id.eq_any(guild_ids.iter().map(|i| i.to_string())))
        .load::<GuildRaceCriteria>(conn)?;
    let mut by_guild_id: HashMap<_, _> = guild_ids
        .iter()
        .map(|id| (**id, vec![]))
        .collect::<HashMap<_, _>>();

    for f in raw_criteria {
        let gid = match f.guild_id.parse::<Id<GuildMarker>>() {
            Ok(g) => g,
            Err(e) => {
                warn!("Unable to parse guild id on GuildRaceCriteria: {f:?} - {e} - deleting");
                f.delete(conn).ok();
                continue;
            }
        };
        by_guild_id.entry(gid).or_insert(vec![]).push(f);
    }
    Ok(by_guild_id
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                GuildCriteria {
                    guild_id: k,
                    criteria: v,
                },
            )
        })
        .collect::<HashMap<_, _>>())
}
