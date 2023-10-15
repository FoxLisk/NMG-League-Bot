use crate::models::bracket_races::BracketRace;
use crate::schema::{bracket_race_infos, commentator_signups};
use std::num::ParseIntError;
use std::str::FromStr;

use crate::{save_fn, update_fn};
use chrono::{DateTime, TimeZone, Utc};
use diesel::prelude::*;
use diesel::SqliteConnection;
use serde::Serialize;
use twilight_mention::timestamp::{Timestamp as MentionTimestamp, TimestampStyle};
use twilight_mention::Mention;
use twilight_model::id::marker::{MessageMarker, ScheduledEventMarker, UserMarker};
use twilight_model::id::Id;

#[derive(Debug, Clone)]
pub struct BracketRaceInfoId(pub i32);

#[derive(Queryable, Identifiable, Debug, AsChangeset, Serialize, Clone)]
#[diesel(treat_none_as_null = true)]
pub struct BracketRaceInfo {
    pub id: i32,
    pub bracket_race_id: i32,
    pub scheduled_for: Option<i64>,
    pub scheduled_event_id: Option<String>,
    pub commportunities_message_id: Option<String>,
    pub restream_request_message_id: Option<String>,
    pub racetime_gg_url: Option<String>,
    pub tentative_commentary_assignment_message_id: Option<String>,
    pub commentary_assignment_message_id: Option<String>,
    pub restream_channel: Option<String>,
}

impl BracketRaceInfo {
    pub fn get_by_id(id: i32, conn: &mut SqliteConnection) -> Result<Self, diesel::result::Error> {
        bracket_race_infos::table
            .filter(bracket_race_infos::id.eq(id))
            .first(conn)
    }
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

    pub fn get_by_commportunities_message_id(
        id: Id<MessageMarker>,
        conn: &mut SqliteConnection,
    ) -> Result<Option<Self>, diesel::result::Error> {
        bracket_race_infos::table
            .filter(bracket_race_infos::commportunities_message_id.eq(id.to_string()))
            .first(conn)
            .optional()
    }

    pub fn get_by_restream_request_message_id(
        id: Id<MessageMarker>,
        conn: &mut SqliteConnection,
    ) -> Result<Option<Self>, diesel::result::Error> {
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
        let short =
            MentionTimestamp::new(ts.timestamp() as u64, Some(TimestampStyle::RelativeTime));
        Some(format!("{} ({})", long.mention(), short.mention()))
    }

    pub fn scheduled(&self) -> Option<DateTime<Utc>> {
        self.scheduled_for
            .map(|t| Utc.timestamp_opt(t, 0).earliest())
            .flatten()
    }

    /// returns the prior scheduled time, if any (as timestamp)
    /// Deletes any existing commentary signups
    /// Cleans self.racetime_gg_url as well
    /// does *not* persist self
    pub fn schedule<T: TimeZone>(
        &mut self,
        when: &DateTime<T>,
        conn: &mut SqliteConnection,
    ) -> Result<Option<i64>, diesel::result::Error> {
        diesel::delete(
            commentator_signups::table
                .filter(commentator_signups::bracket_race_info_id.eq(self.id)),
        )
        .execute(conn)?;
        self.racetime_gg_url = None;
        Ok(std::mem::replace(
            &mut self.scheduled_for,
            Some(when.timestamp()),
        ))
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
        attr_id_to_real_id(&self.commportunities_message_id)
    }

    pub fn clear_commportunities_message_id(&mut self) {
        self.commportunities_message_id = None;
    }

    /// Returns the old scheduled event ID, if any
    /// (it's a string b/c sqlite)
    pub fn set_commportunities_message_id(&mut self, id: Id<MessageMarker>) -> Option<String> {
        std::mem::replace(&mut self.commportunities_message_id, Some(id.to_string()))
    }

    /// Returns the old restream request post ID, if any
    /// (it's a string b/c sqlite)
    pub fn set_restream_request_message_id(&mut self, id: Id<MessageMarker>) -> Option<String> {
        std::mem::replace(&mut self.restream_request_message_id, Some(id.to_string()))
    }

    /// Returns the old tentative commentary assignment post ID, if any
    /// (it's a string b/c sqlite)
    pub fn set_tentative_commentary_assignment_message_id(
        &mut self,
        id: Id<MessageMarker>,
    ) -> Option<String> {
        std::mem::replace(
            &mut self.tentative_commentary_assignment_message_id,
            Some(id.to_string()),
        )
    }

    pub fn get_tentative_commentary_assignment_message_id(&self) -> Option<Id<MessageMarker>> {
        attr_id_to_real_id(&self.tentative_commentary_assignment_message_id)
    }

    pub fn clear_tentative_commentary_assignment_message_id(&mut self) {
        self.tentative_commentary_assignment_message_id = None;
    }

    /// Returns the old tentative commentary assignment post ID, if any
    /// (it's a string b/c sqlite)
    pub fn set_commentary_assignment_message_id(
        &mut self,
        id: Id<MessageMarker>,
    ) -> Option<String> {
        std::mem::replace(
            &mut self.commentary_assignment_message_id,
            Some(id.to_string()),
        )
    }

    /// * true if the save succeed,
    /// * false if it failed for unique constraint violation,
    /// * err if any other error occurred
    pub fn new_commentator_signup(
        &mut self,
        user_id: Id<UserMarker>,
        conn: &mut SqliteConnection,
    ) -> Result<bool, diesel::result::Error> {
        let nsi = NewCommentatorSignup::new(self, user_id);
        match nsi.save(conn) {
            Ok(_) => Ok(true),
            Err(diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                _,
            )) => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub fn remove_commentator(
        &mut self,
        user_id: Id<UserMarker>,
        conn: &mut SqliteConnection,
    ) -> Result<usize, diesel::result::Error> {
        diesel::delete(
            commentator_signups::table
                .filter(commentator_signups::discord_id.eq(user_id.to_string()))
                .filter(commentator_signups::bracket_race_info_id.eq(self.id)),
        )
        .execute(conn)
    }

    pub fn commentator_signups(
        &self,
        conn: &mut SqliteConnection,
    ) -> Result<Vec<CommentatorSignup>, diesel::result::Error> {
        commentator_signups::table
            .filter(commentator_signups::bracket_race_info_id.eq(self.id))
            .load(conn)
    }

    update_fn! {}
}

fn attr_id_to_real_id<T>(id: &Option<String>) -> Option<Id<T>> {
    id.as_ref()
        .map(|id| Id::from_str(id.as_str()).ok())
        .flatten()
}

#[derive(Insertable)]
#[diesel(table_name=bracket_race_infos)]
pub struct NewBracketRaceInfo {
    bracket_race_id: i32,
    scheduled_for: Option<i64>,
    scheduled_event_id: Option<String>,
    commportunities_message_id: Option<String>,
    restream_request_message_id: Option<String>,
    racetime_gg_url: Option<String>,
    tentative_commentary_assignment_message_id: Option<String>,
    commentary_assignment_message_id: Option<String>,
    restream_channel: Option<String>,
}

impl NewBracketRaceInfo {
    pub fn new(bracket_race: &BracketRace) -> Self {
        Self {
            bracket_race_id: bracket_race.id,
            scheduled_for: None,
            scheduled_event_id: None,
            commportunities_message_id: None,
            restream_request_message_id: None,
            racetime_gg_url: None,
            tentative_commentary_assignment_message_id: None,
            commentary_assignment_message_id: None,
            restream_channel: None,
        }
    }

    save_fn!(bracket_race_infos::table, BracketRaceInfo);
}

#[derive(Queryable, Debug, Serialize)]
pub struct CommentatorSignup {
    id: i32,
    bracket_race_info_id: i32,
    discord_id: String,
}

impl CommentatorSignup {
    pub fn discord_id(&self) -> Result<Id<UserMarker>, ParseIntError> {
        Id::from_str(&self.discord_id)
    }
}

#[derive(Insertable)]
#[diesel(table_name=commentator_signups)]
pub struct NewCommentatorSignup {
    bracket_race_info_id: i32,
    discord_id: String,
}

impl NewCommentatorSignup {
    fn new(bri: &BracketRaceInfo, discord_id: Id<UserMarker>) -> Self {
        Self {
            bracket_race_info_id: bri.id,
            discord_id: discord_id.to_string(),
        }
    }

    save_fn!(commentator_signups::table, CommentatorSignup);
}
