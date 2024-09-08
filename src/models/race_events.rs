use std::str::FromStr;

use diesel::prelude::*;
use serde::Serialize;
use twilight_model::id::{
    marker::{GuildMarker, ScheduledEventMarker},
    Id,
};

use crate::{save_fn, schema::race_events, update_fn};

use super::bracket_race_infos::{BracketRaceInfo, BracketRaceInfoId};

#[derive(Queryable, Identifiable, Debug, AsChangeset, Serialize, Clone, Selectable)]
#[diesel(treat_none_as_null = true)]
/// This model represents our knowledge of a *Discord event* in a particular guild for a particular race
pub struct RaceEvent {
    pub id: i32,
    pub guild_id: String,
    pub bracket_race_info_id: i32,
    pub scheduled_event_id: String,
}

impl RaceEvent {
    /// returns all `RaceEvent`s associated with any of these BRI Ids
    ///
    /// result is sorted by guild id
    pub fn get_for_bri_ids(
        ids: &[BracketRaceInfoId],
        conn: &mut SqliteConnection,
    ) -> Result<Vec<Self>, diesel::result::Error> {
        race_events::table
            .filter(
                race_events::dsl::bracket_race_info_id.eq_any(ids.iter().map(|bri_id| bri_id.0)),
            )
            .order_by(race_events::dsl::guild_id)
            .load(conn)
    }
}

impl RaceEvent {
    pub fn get_scheduled_event_id(&self) -> Option<Id<ScheduledEventMarker>> {
        Id::from_str(&self.scheduled_event_id).ok()
    }

    pub fn set_scheduled_event_id(&mut self, id: Id<ScheduledEventMarker>) -> String {
        std::mem::replace(&mut self.scheduled_event_id, id.to_string())
    }

    update_fn!();
}

#[derive(Insertable)]
#[diesel(table_name=race_events)]
pub struct NewRaceEvent {
    guild_id: String,
    bracket_race_info_id: i32,
    scheduled_event_id: String,
}

impl NewRaceEvent {
    pub fn new(
        guild_id: Id<GuildMarker>,
        bri: &BracketRaceInfo,
        scheduled_event_id: Id<ScheduledEventMarker>,
    ) -> Self {
        Self {
            guild_id: guild_id.to_string(),
            bracket_race_info_id: bri.id,
            scheduled_event_id: scheduled_event_id.to_string(),
        }
    }
    save_fn!(race_events::table, RaceEvent);
}
