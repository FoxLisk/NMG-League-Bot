use std::str::FromStr;
use crate::models::bracket_races::BracketRace;
use crate::schema::bracket_race_infos;
use crate::{save_fn, update_fn};
use chrono::{DateTime, TimeZone, Utc};
use diesel::prelude::*;
use diesel::SqliteConnection;
use serde::Serialize;
use twilight_mention::Mention;
use twilight_mention::timestamp::{Timestamp as MentionTimestamp, TimestampStyle};
use twilight_model::channel::Message;
use twilight_model::id::Id;
use twilight_model::id::marker::{MessageMarker, ScheduledEventMarker, WebhookMarker};
use twilight_model::util::Timestamp;


#[derive(Queryable, Identifiable, Debug, AsChangeset, Serialize)]
#[changeset_options(treat_none_as_null="true")]
pub struct BracketRaceInfo {
    id: i32,
    bracket_race_id: i32,
    scheduled_for: Option<i64>,
    scheduled_event_id: Option<String>,
    commportunities_message_id : Option<String>,
    restream_request_message_id : Option<String>,
}

impl BracketRaceInfo {
    pub fn get_or_create_for_bracket(
        bracket_race: &BracketRace,
        conn: &mut SqliteConnection,
    ) -> Result<Self, diesel::result::Error> {
        conn.transaction(|conn| {
            match bracket_race_infos::table
                .filter(bracket_race_infos::bracket_race_id.eq(bracket_race.id))
                .first(conn)
                .optional()?
            {
                Some(bi) => Ok(bi),
                None => NewBracketRaceInfo::new(bracket_race).save(conn),
            }
        })
    }

    pub fn get_by_commportunities_message_id(id: Id<MessageMarker>, conn: &mut SqliteConnection) -> Result<Option<Self>, diesel::result::Error> {
        bracket_race_infos::table
            .filter(bracket_race_infos::commportunities_message_id.eq(id.to_string()))
            .first(conn)
            .optional()
    }


    pub fn get_by_restream_request_message_id(id: Id<MessageMarker>, conn: &mut SqliteConnection) -> Result<Option<Self>, diesel::result::Error> {
        bracket_race_infos::table
            .filter(bracket_race_infos::restream_request_message_id.eq(id.to_string()))
            .first(conn)
            .optional()
    }

    pub fn race(&self, conn: &mut SqliteConnection) -> Result<BracketRace, diesel::result::Error> {
        BracketRace::get_by_id(self.bracket_race_id, conn)
    }


    /// returns a string like "<Long Date And Time> (<Relative time>)" if scheduled
    pub fn scheduled_time_formatted(&self) -> Option<String> {
        let ts = self.scheduled()?;
        let long = MentionTimestamp::new(ts.timestamp() as u64, Some(TimestampStyle::LongDateTime));
        let short = MentionTimestamp::new(ts.timestamp() as u64, Some(TimestampStyle::RelativeTime));
        Some(format!("{} ({})", long.mention(), short.mention()))

    }

    pub fn scheduled(&self) -> Option<DateTime<Utc>> {
        self.scheduled_for.map(|t| Utc.timestamp(t, 0))
    }


    /// returns the prior scheduled time, if any (as timestamp)
    pub fn schedule<T: TimeZone>(
        &mut self,
        when: &DateTime<T>,
    ) -> Option<i64> {
        std::mem::replace(&mut self.scheduled_for, Some(when.timestamp()))
    }

    pub fn get_scheduled_event_id(&self) -> Option<Id<ScheduledEventMarker>> {
        attr_id_to_real_id(&self.scheduled_event_id)
    }

    /// Returns the old scheduled event ID, if any
    /// (it's a string b/c sqlite)
    pub fn set_scheduled_event_id(&mut self, id: Id<ScheduledEventMarker>) -> Option<String> {
        std::mem::replace(&mut self.scheduled_event_id, Some(id.to_string()))
    }

    pub fn get_commportunities_message_id(&self) -> Option<Id<MessageMarker>> {
        attr_id_to_real_id(&self.commportunities_message_id )
    }

    pub fn clear_commportunities_message_id(&mut self) {
        self.commportunities_message_id = None;
    }

    /// Returns the old scheduled event ID, if any
    /// (it's a string b/c sqlite)
    pub fn set_commportunities_message_id(&mut self, id: Id<MessageMarker>) -> Option<String> {
        std::mem::replace(&mut self.commportunities_message_id , Some(id.to_string()))
    }

    /// Returns the old restream request post ID, if any
    /// (it's a string b/c sqlite)
    pub fn set_restream_request_message_id(&mut self, id: Id<MessageMarker>) -> Option<String> {
        std::mem::replace(&mut self.restream_request_message_id , Some(id.to_string()))
    }

    update_fn! {}
}

fn attr_id_to_real_id<T>(id: &Option<String>) -> Option<Id<T>> {
    id.as_ref()
        .map(|id|
            Id::from_str(id.as_str())
                .ok()
        ).flatten()
}

#[derive(Insertable)]
#[diesel(table_name=bracket_race_infos)]
pub struct NewBracketRaceInfo {
    bracket_race_id: i32,
    scheduled_for: Option<i64>,
    scheduled_event_id: Option<String>,
    commportunities_message_id: Option<String>,
    restream_request_message_id: Option<String>,
}

impl NewBracketRaceInfo {
    pub fn new(bracket_race: &BracketRace) -> Self {
        Self {
            bracket_race_id: bracket_race.id,
            scheduled_for: None,
            scheduled_event_id: None,
            commportunities_message_id: None,
            restream_request_message_id: None,
        }
    }

    save_fn!(bracket_race_infos::table, BracketRaceInfo);
}
