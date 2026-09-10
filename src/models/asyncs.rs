use std::num::ParseIntError;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AsyncRaceError {
    #[error("database error: {0}")]
    Database(#[from] diesel::result::Error),

    #[error("an asynchronous race must have two different racers")]
    DuplicateRacer,

    #[error("expected exactly two runs for race {race_id}, found {actual}")]
    UnexpectedRunCount { race_id: i32, actual: usize },

    #[error("no race run found for Discord message {message_id}")]
    RunNotFound { message_id: u64 },

    #[error("invalid Discord racer id {value:?}: {source}")]
    InvalidRacerId {
        value: String,
        #[source]
        source: ParseIntError,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FilenameParseError {
    #[error("invalid filenames value {value:?}: expected formats like `A BCD EFGH`")]
    InvalidFormat { value: String },
}

pub mod race {
    use super::AsyncRaceError;
    use crate::models::asyncs::race_run::{AsyncRaceRun, NewAsyncRaceRun, RaceRunState};
    use crate::save_fn;
    use crate::schema::races;
    use crate::utils::{epoch_timestamp, uuid_string};
    use diesel::prelude::{AsChangeset, Identifiable, Insertable, Queryable};
    use diesel::sql_types::Text;
    use diesel::{AsExpression, RunQueryDsl, SqliteConnection};
    use diesel_enum_derive::DieselEnum;
    use serde::Serialize;
    use twilight_model::id::marker::UserMarker;
    use twilight_model::id::Id;

    #[derive(Debug, Serialize, PartialEq, Clone, DieselEnum, AsExpression)]
    #[allow(non_camel_case_types)]
    #[diesel(sql_type = Text)]
    pub enum RaceState {
        CREATED,
        FINISHED,
        ABANDONED,
        CANCELLED_BY_ADMIN,
    }

    #[derive(Insertable)]
    #[diesel(table_name=crate::schema::races)]
    pub struct NewAsyncRace {
        uuid: String,
        created: i64,
        state: String,
        on_start_message: Option<String>,
    }

    #[derive(Queryable, Clone, Identifiable)]
    #[diesel(table_name = crate::schema::races)]
    pub struct AsyncRace {
        pub id: i32,
        pub uuid: String,
        #[diesel(deserialize_as=i64)]
        pub created: u32,
        #[diesel(deserialize_as=String)]
        pub state: RaceState,
        pub on_start_message: Option<String>,
    }

    #[derive(Identifiable, AsChangeset)]
    #[diesel(table_name = crate::schema::races)]
    struct UpdateRace {
        id: i32,
        uuid: String,
        created: i64,
        state: String,
        on_start_message: Option<String>,
    }

    impl From<AsyncRace> for UpdateRace {
        fn from(r: AsyncRace) -> Self {
            Self {
                id: r.id,
                uuid: r.uuid,
                created: r.created as i64,
                state: r.state.into(),
                on_start_message: r.on_start_message,
            }
        }
    }

    // statics
    impl AsyncRace {
        pub fn get_by_id(
            id_: i32,
            conn: &mut SqliteConnection,
        ) -> Result<Self, diesel::result::Error> {
            use crate::schema::races::dsl::*;
            use diesel::prelude::*;
            races.filter(id.eq(id_)).first(conn)
        }
    }

    impl AsyncRace {
        pub fn finish(&mut self) {
            self.state = RaceState::FINISHED;
        }

        pub fn abandon(&mut self) {
            self.state = RaceState::ABANDONED;
        }

        /// clones self
        pub async fn save(&self, conn: &mut SqliteConnection) -> Result<(), diesel::result::Error> {
            let update = UpdateRace::from(self.clone());
            diesel::update(&update)
                .set(&update)
                .execute(conn)
                .map(|_| ())
        }

        async fn add_run(
            &self,
            racer_id: Id<UserMarker>,
            cxn: &mut SqliteConnection,
        ) -> Result<AsyncRaceRun, diesel::result::Error> {
            let nrr = NewAsyncRaceRun::new(self.id, racer_id);
            diesel::insert_into(crate::schema::race_runs::table)
                .values(nrr)
                .get_result(cxn)
        }

        /// Creates RaceRuns with the appropriate users and associates them with this race
        pub async fn select_racers(
            &self,
            racer_1: Id<UserMarker>,
            racer_2: Id<UserMarker>,
            cxn: &mut SqliteConnection,
        ) -> Result<(AsyncRaceRun, AsyncRaceRun), AsyncRaceError> {
            if racer_1 == racer_2 {
                return Err(AsyncRaceError::DuplicateRacer);
            }
            let r1 = self.add_run(racer_1, cxn).await?;
            let r2 = self.add_run(racer_2, cxn).await?;
            Ok((r1, r2))
        }

        /// Cancels this race and its associated RaceRun
        /// This updates the database
        pub async fn cancel(
            &self,
            conn: &mut SqliteConnection,
        ) -> Result<(), diesel::result::Error> {
            use crate::schema::race_runs::dsl::{race_id, state as run_state};
            use crate::schema::races::dsl::state;
            use diesel::prelude::*;

            conn.transaction(|conn| {
                diesel::update(self)
                    .set(state.eq(String::from(RaceState::CANCELLED_BY_ADMIN)))
                    .execute(conn)
                    .map(|_| ())?;
                diesel::update(crate::schema::race_runs::table.filter(race_id.eq(self.id)))
                    .set(run_state.eq(String::from(RaceRunState::CANCELLED_BY_ADMIN)))
                    .execute(conn)
                    .map(|_| ())
            })
        }

        pub async fn get_runs(
            &self,
            conn: &mut SqliteConnection,
        ) -> Result<(AsyncRaceRun, AsyncRaceRun), AsyncRaceError> {
            AsyncRaceRun::get_runs(&self, conn).await
        }
    }

    impl NewAsyncRace {
        pub fn new(on_start_message: Option<String>) -> Self {
            Self {
                uuid: uuid_string(),
                created: epoch_timestamp() as i64,
                state: RaceState::CREATED.into(),
                on_start_message,
            }
        }

        save_fn!(races::table, AsyncRace);
    }
}

pub mod race_run {
    use super::{AsyncRaceError, FilenameParseError};
    use crate::models::asyncs::race::AsyncRace;
    use crate::schema::race_runs;
    use crate::utils::epoch_timestamp;
    use crate::utils::uuid_string;
    use crate::utils::{format_duration_hms, time_delta_lifted, timestamp_to_naivedatetime};
    use chrono::NaiveDateTime;
    use diesel::prelude::*;
    use diesel::sql_types::Text;
    use diesel::AsExpression;
    use diesel_enum_derive::DieselEnum;
    use lazy_static::lazy_static;
    use rand::rngs::ThreadRng;
    use rand::{thread_rng, Rng};
    use serde::Serialize;
    use std::fmt::Formatter;
    use std::str::FromStr;
    use twilight_model::id::marker::{MessageMarker, UserMarker};
    use twilight_model::id::Id;
    lazy_static! {
        static ref FILENAMES_REGEX: regex::Regex =
            regex::Regex::new("^([A-Z]) ([A-Z]{3}) ([A-Z]{4})$").unwrap();
    }

    pub struct Filenames {
        pub one: char,
        pub three: [char; 3],
        pub four: [char; 4],
    }

    fn random_char(rng: &mut ThreadRng) -> char {
        rng.gen_range('A'..='Z')
    }

    impl std::fmt::Display for Filenames {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.to_str())
        }
    }

    impl Filenames {
        // TODO: find a list of words to filter or whatever
        fn _validate_four(&self) -> bool {
            match self.four {
                ['c', 'u', 'n', 't'] => false,
                _ => true,
            }
        }

        fn _validate_three(&self) -> bool {
            match self.three {
                ['f', 'a', 'g'] => false,
                _ => true,
            }
        }

        fn _validate(&self) -> bool {
            self._validate_three() && self._validate_four()
        }

        fn _new_random() -> Self {
            let mut rng = thread_rng();
            let one = random_char(&mut rng);
            let three = [
                random_char(&mut rng),
                random_char(&mut rng),
                random_char(&mut rng),
            ];
            let four = [
                random_char(&mut rng),
                random_char(&mut rng),
                random_char(&mut rng),
                random_char(&mut rng),
            ];

            Self { one, three, four }
        }

        pub fn new_random() -> Self {
            loop {
                let f = Self::_new_random();
                if f._validate() {
                    return f;
                }
            }
        }
    }

    impl FromStr for Filenames {
        type Err = FilenameParseError;

        fn from_str(value: &str) -> Result<Self, FilenameParseError> {
            let invalid = || FilenameParseError::InvalidFormat {
                value: value.to_owned(),
            };
            let caps = FILENAMES_REGEX.captures(value).ok_or_else(invalid)?;
            let one = caps
                .get(1)
                .and_then(|capture| capture.as_str().chars().next())
                .ok_or_else(invalid)?;
            let three = caps
                .get(2)
                .map(|capture| capture.as_str().chars().collect::<Vec<_>>())
                .and_then(|chars| chars.try_into().ok())
                .ok_or_else(invalid)?;
            let four = caps
                .get(3)
                .map(|capture| capture.as_str().chars().collect::<Vec<_>>())
                .and_then(|chars| chars.try_into().ok())
                .ok_or_else(invalid)?;

            Ok(Self { one, three, four })
        }
    }

    impl Filenames {
        fn to_str(&self) -> String {
            let mut s = String::with_capacity(10);
            s.push(self.one);
            s.push(' ');
            s.extend(self.three);
            s.push(' ');
            s.extend(self.four);
            s
        }
    }

    impl From<Filenames> for String {
        fn from(fns: Filenames) -> Self {
            fns.to_str()
        }
    }

    #[derive(Debug, Serialize, PartialEq, DieselEnum, Clone, AsExpression)]
    #[allow(non_camel_case_types)]
    #[diesel(sql_type = Text)]
    pub enum RaceRunState {
        /// RaceRun created
        CREATED,
        /// Player successfully contacted via DM
        CONTACTED,
        /// Player has clicked Start run
        STARTED,
        /// Player has clicked Finish
        FINISHED,
        /// Player has submitted their run time (or, well, some text in that input box)
        TIME_SUBMITTED,
        /// Player has submitted VoD link (or, well, some text in that input box)
        VOD_SUBMITTED,
        /// Player has confirmed forfeit
        FORFEIT,
        /// Associated race was cancelled
        // N.B. maybe this shouldn't be denormalized and we should just check
        // race state on every RaceRun operation?
        CANCELLED_BY_ADMIN,
    }

    impl RaceRunState {
        pub fn is_created(&self) -> bool {
            match self {
                Self::CREATED => true,
                _ => false,
            }
        }

        pub fn is_pre_start(&self) -> bool {
            match self {
                Self::CREATED | Self::CONTACTED => true,
                _ => false,
            }
        }
    }

    #[derive(Clone, Queryable, Identifiable)]
    #[diesel(table_name = crate::schema::race_runs)]
    pub struct AsyncRaceRun {
        pub id: i32,
        pub uuid: String,
        race_id: i32,
        racer_id: String,
        filenames: String,
        #[diesel(deserialize_as=i64)]
        created: u32,
        #[diesel(deserialize_as=String)]
        // can we get away with making this private?
        pub state: RaceRunState,
        pub run_started: Option<i64>,
        pub run_finished: Option<i64>,
        pub reported_run_time: Option<String>,
        reported_at: Option<i64>,
        pub message_id: Option<String>,
        pub vod: Option<String>,
    }

    #[derive(Identifiable, AsChangeset)]
    #[diesel(table_name = crate::schema::race_runs)]
    struct UpdateAsyncRaceRun {
        id: i32,
        uuid: String,
        race_id: i32,
        racer_id: String,
        filenames: String,
        created: i64,
        state: String,
        run_started: Option<i64>,
        run_finished: Option<i64>,
        reported_run_time: Option<String>,
        reported_at: Option<i64>,
        message_id: Option<String>,
        vod: Option<String>,
    }

    impl From<AsyncRaceRun> for UpdateAsyncRaceRun {
        fn from(rr: AsyncRaceRun) -> Self {
            Self {
                id: rr.id,
                uuid: rr.uuid,
                race_id: rr.race_id,
                racer_id: rr.racer_id,
                filenames: rr.filenames,
                created: rr.created as i64,
                state: String::from(rr.state),
                run_started: rr.run_started,
                run_finished: rr.run_finished,
                reported_run_time: rr.reported_run_time,
                reported_at: rr.reported_at,
                message_id: rr.message_id,
                vod: rr.vod,
            }
        }
    }

    // statics
    impl AsyncRaceRun {
        pub async fn get_runs(
            race: &AsyncRace,
            conn: &mut SqliteConnection,
        ) -> Result<(AsyncRaceRun, AsyncRaceRun), AsyncRaceError> {
            use crate::schema::race_runs::dsl::*;
            let mut runs: Vec<AsyncRaceRun> = race_runs.filter(race_id.eq(race.id)).load(conn)?;
            if runs.len() == 2 {
                Ok((runs.pop().unwrap(), runs.pop().unwrap()))
            } else {
                Err(AsyncRaceError::UnexpectedRunCount {
                    race_id: race.id,
                    actual: runs.len(),
                })
            }
        }

        pub async fn get_by_message_id(
            message_id: Id<MessageMarker>,
            conn: &mut SqliteConnection,
        ) -> Result<Self, AsyncRaceError> {
            Self::search_by_message_id(message_id.clone(), conn)
                .await
                .and_then(|rr| {
                    rr.ok_or(AsyncRaceError::RunNotFound {
                        message_id: message_id.get(),
                    })
                })
        }

        pub async fn search_by_message_id(
            message_id_: Id<MessageMarker>,
            conn: &mut SqliteConnection,
        ) -> Result<Option<Self>, AsyncRaceError> {
            use crate::schema::race_runs::dsl::*;
            let mut runs: Vec<Self> = race_runs
                .filter(message_id.eq(message_id_.to_string()))
                .load(conn)?;
            Ok(runs.pop())
        }
    }

    // instance
    impl AsyncRaceRun {
        pub fn racer_id(&self) -> Result<Id<UserMarker>, AsyncRaceError> {
            self.racer_id.parse::<Id<UserMarker>>().map_err(|source| {
                AsyncRaceError::InvalidRacerId {
                    value: self.racer_id.clone(),
                    source,
                }
            })
        }

        pub fn is_finished(&self) -> bool {
            match self.state {
                RaceRunState::VOD_SUBMITTED => true,
                RaceRunState::FORFEIT => true,
                _ => false,
            }
        }

        pub fn filenames(&self) -> Result<Filenames, FilenameParseError> {
            self.filenames.parse()
        }

        pub fn contact_succeeded(&mut self) {
            self.state = RaceRunState::CONTACTED;
        }

        pub fn get_message_id(&self) -> Option<Id<MessageMarker>> {
            self.message_id
                .as_ref()
                .and_then(|ms| Id::<MessageMarker>::from_str(&ms).ok())
        }

        pub fn get_started_at(&self) -> Option<NaiveDateTime> {
            self.run_started.map(timestamp_to_naivedatetime)
        }

        pub fn get_finished_at(&self) -> Option<NaiveDateTime> {
            self.run_finished.map(timestamp_to_naivedatetime)
        }

        pub fn get_time_to_finish(&self) -> Option<String> {
            time_delta_lifted(self.get_started_at(), self.get_finished_at())
                .map(format_duration_hms)
        }

        pub fn get_time_from_finish_to_report(&self) -> Option<String> {
            time_delta_lifted(self.get_finished_at(), self.get_reported_at())
                .map(format_duration_hms)
        }

        pub fn get_reported_at(&self) -> Option<NaiveDateTime> {
            self.reported_at
                .map(|t| NaiveDateTime::from_timestamp_opt(t, 0))
                .flatten()
        }

        /// Sets the models state to started
        pub fn start(&mut self) {
            self.state = RaceRunState::STARTED;
            self.run_started = Some(epoch_timestamp() as i64);
        }

        /// If the run is already finished this does *not* update it
        pub fn finish(&mut self) {
            self.state = RaceRunState::FINISHED;
            if self.run_finished.is_none() {
                self.run_finished = Some(epoch_timestamp() as i64);
            }
        }

        pub fn forfeit(&mut self) {
            self.state = RaceRunState::FORFEIT;
        }

        pub fn report_user_time(&mut self, user_time: String) {
            self.state = RaceRunState::TIME_SUBMITTED;
            self.reported_at = Some(epoch_timestamp().into());
            self.reported_run_time = Some(user_time);
        }

        pub fn set_vod(&mut self, vod: String) {
            self.state = RaceRunState::VOD_SUBMITTED;
            self.vod = Some(vod);
        }

        pub async fn save(&self, conn: &mut SqliteConnection) -> Result<(), diesel::result::Error> {
            let update = UpdateAsyncRaceRun::from(self.clone());
            diesel::update(self).set(update).execute(conn).map(|_| ())
        }

        pub fn set_message_id(&mut self, message_id: u64) {
            self.message_id = Some(message_id.to_string());
        }

        pub fn get_race(
            &self,
            conn: &mut SqliteConnection,
        ) -> Result<AsyncRace, diesel::result::Error> {
            AsyncRace::get_by_id(self.race_id, conn)
        }
    }

    // It is impossible in Rust to `impl Into<String> for Id<UserMarker>`
    // That means we can't insert a struct with an `Id<UserMarker>` in it, so we have to
    // convert it ourselves.

    #[derive(Insertable)]
    #[diesel(table_name=race_runs)]
    pub struct NewAsyncRaceRun {
        race_id: i32,
        uuid: String,
        racer_id: String,
        #[diesel(serialize_as=String)]
        filenames: Filenames,
        #[diesel(serialize_as=i64)]
        created: u32,
        #[diesel(serialize_as=String)]
        state: RaceRunState,
    }

    impl NewAsyncRaceRun {
        pub fn new(race_id: i32, racer_id: Id<UserMarker>) -> Self {
            Self {
                race_id,
                uuid: uuid_string(),
                racer_id: racer_id.to_string(),
                filenames: Filenames::new_random(),
                created: epoch_timestamp(),
                state: RaceRunState::CREATED,
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::Filenames;

        #[test]
        fn filenames_parse_valid_value() {
            let filenames: Filenames = "A BCD EFGH".parse().unwrap();
            assert_eq!("A BCD EFGH", filenames.to_string());
        }

        #[test]
        fn filenames_reject_surrounding_garbage() {
            assert!("prefix A BCD EFGH suffix".parse::<Filenames>().is_err());
        }
    }
}
