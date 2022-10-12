use diesel::{Insertable, QueryResult, RunQueryDsl, SqliteConnection, Table};

pub mod player;
pub mod season;
pub mod brackets;
pub mod bracket_races;
pub mod player_bracket_entries;

pub(crate) fn uuid_string() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(crate) fn epoch_timestamp() -> u32 {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let t_u32 = timestamp as u32;
    if t_u32 as u64 != timestamp {
        println!(
            "Error: timestamp too big?? got {} secs since epoch, which converted to {}",
            timestamp, t_u32
        );
    }
    t_u32
}

// TODO: should this be a derive macro?
#[macro_export]
macro_rules! save_fn {
    ($table:expr, $output:ty) => {
        pub fn save(&self, cxn: &mut diesel::SqliteConnection) -> diesel::QueryResult<$output> {
            use diesel::RunQueryDsl;
            diesel::insert_into($table)
                .values(self)
                .get_result(cxn)
        }
    }
}

pub(crate) mod race {
    use crate::models::race_run::{NewRaceRun, RaceRun, RaceRunState};
    use crate::models::{epoch_timestamp, uuid_string};
    use crate::schema::races;
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
    pub(crate) enum RaceState {
        CREATED,
        FINISHED,
        ABANDONED,
        CANCELLED_BY_ADMIN,
    }

    #[derive(Insertable)]
    #[diesel(table_name=races)]
    pub(crate) struct NewRace {
        uuid: String,
        created: i64,
        #[diesel(serialize_as = String)]
        state: RaceState,
    }

    #[derive(Queryable, Clone, Identifiable)]
    pub(crate) struct Race {
        pub(crate) id: i32,
        pub(crate) uuid: String,
        #[diesel(deserialize_as=i64)]
        pub(crate) created: u32,
        #[diesel(deserialize_as=String)]
        pub(crate) state: RaceState,
    }

    #[derive(Identifiable, AsChangeset)]
    #[table_name = "races"]
    struct UpdateRace {
        id: i32,
        uuid: String,
        created: i64,
        state: String,
    }

    impl From<Race> for UpdateRace {
        fn from(r: Race) -> Self {
            Self {
                id: r.id,
                uuid: r.uuid,
                created: r.created as i64,
                state: r.state.into(),
            }
        }
    }

    // statics
    impl Race {
        pub(crate) fn get_by_id(
            id_: i32,
            conn: &mut SqliteConnection,
        ) -> Result<Self, String> {
            use crate::schema::races::dsl::*;
            use diesel::prelude::*;
            races
                .filter(id.eq(id_))
                .first(conn)
                .map_err(|e| e.to_string())
        }
    }

    impl Race {
        pub(crate) fn finish(&mut self) {
            self.state = RaceState::FINISHED;
        }

        pub(crate) fn abandon(&mut self) {
            self.state = RaceState::ABANDONED;
        }

        /// clones self
        pub(crate) async fn save(&self, conn: &mut SqliteConnection) -> Result<(), String> {
            let update = UpdateRace::from(self.clone());
            diesel::update(&update)
                .set(&update)
                .execute(conn)
                .map_err(|e| e.to_string())
                .map(|_| ())
        }

        async fn add_run(
            &self,
            racer_id: Id<UserMarker>,
            cxn: &mut SqliteConnection,
        ) -> Result<RaceRun, String> {
            let nrr = NewRaceRun::new(self.id, racer_id);
            diesel::insert_into(crate::schema::race_runs::table)
                .values(nrr)
                .get_result(cxn)
                .map_err(|e| format!("Error saving race: {}", e))
        }

        /// Creates RaceRuns with the appropriate users and associates them with this race
        pub(crate) async fn select_racers(
            &self,
            racer_1: Id<UserMarker>,
            racer_2: Id<UserMarker>,
            cxn: &mut SqliteConnection,
        ) -> Result<(RaceRun, RaceRun), String> {
            if racer_1 == racer_2 {
                return Err("Racers must be different users!".to_string());
            }
            let r1 = self.add_run(racer_1, cxn).await?;
            let r2 = self.add_run(racer_2, cxn).await?;
            Ok((r1, r2))
        }

        /// Cancels this race and its associated RaceRun
        /// This updates the database
        pub(crate) async fn cancel(
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

        pub(crate) async fn get_runs(
            &self,
            conn: &mut SqliteConnection,
        ) -> Result<(RaceRun, RaceRun), String> {
            RaceRun::get_runs(self.id, conn).await
        }
    }

    impl NewRace {
        pub(crate) fn new() -> Self {
            Self {
                uuid: uuid_string(),
                created: epoch_timestamp() as i64,
                state: RaceState::CREATED,
            }
        }
    }
}

pub(crate) mod race_run {
    use crate::models::epoch_timestamp;
    use crate::models::uuid_string;
    use crate::schema::race_runs;
    use crate::utils::{format_duration_hms, time_delta_lifted, timestamp_to_naivedatetime};
    use chrono::NaiveDateTime;
    use diesel::prelude::*;
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
            regex::Regex::new("([A-Z]) ([A-Z]{3}) ([A-Z]{4})").unwrap();
    }

    pub(crate) struct Filenames {
        pub(crate) one: char,
        pub(crate) three: [char; 3],
        pub(crate) four: [char; 4],
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

        fn new_random() -> Self {
            loop {
                let f = Self::_new_random();
                if f._validate() {
                    return f;
                }
            }
        }

        fn from_str(value: &str) -> Result<Self, String> {
            let caps = FILENAMES_REGEX
                .captures(value)
                .ok_or(format!("Invalid filenames field: {} - bad format", value))?;
            let one = caps
                .get(1)
                .ok_or(format!("Invalid filenames field 1: {}", value))?;
            let three_cap = caps
                .get(2)
                .ok_or(format!("Invalid filenames field 3: {}", value))?;
            let four_cap = caps
                .get(3)
                .ok_or(format!("Invalid filenames field 4: {}", value))?;

            if three_cap.as_str().len() != 3 {
                return Err(format!("Invalid filenames field 3: {}", three_cap.as_str()));
            }
            let mut three_chars = three_cap.as_str().chars();

            let three: [char; 3] = [
                three_chars.next().unwrap(),
                three_chars.next().unwrap(),
                three_chars.next().unwrap(),
            ];

            if four_cap.as_str().len() != 4 {
                return Err(format!("Invalid filenames field 4: {}", four_cap.as_str()));
            }
            let mut four_chars = four_cap.as_str().chars();
            let four: [char; 4] = [
                four_chars.next().unwrap(),
                four_chars.next().unwrap(),
                four_chars.next().unwrap(),
                four_chars.next().unwrap(),
            ];

            if one.as_str().len() != 1 {
                return Err(format!("Invalid filenames field 1: {}", one.as_str()));
            }

            Ok(Self {
                one: one.as_str().chars().next().unwrap(),
                three,
                four,
            })
        }

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

    #[derive(Debug, Serialize, PartialEq, DieselEnum, Clone)]
    #[allow(non_camel_case_types)]
    pub(crate) enum RaceRunState {
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
        pub(crate) fn is_created(&self) -> bool {
            match self {
                Self::CREATED => true,
                _ => false,
            }
        }

        pub(crate) fn is_pre_start(&self) -> bool {
            match self {
                Self::CREATED | Self::CONTACTED => true,
                _ => false,
            }
        }
    }

    #[derive(Clone, Queryable, Identifiable)]
    pub(crate) struct RaceRun {
        pub(crate) id: i32,
        pub(crate) uuid: String,
        race_id: i32,
        racer_id: String,
        filenames: String,
        #[diesel(deserialize_as=i64)]
        created: u32,
        #[diesel(deserialize_as=String)]
        pub(crate) state: RaceRunState,
        pub(crate) run_started: Option<i64>,
        pub(crate) run_finished: Option<i64>,
        pub(crate) reported_run_time: Option<String>,
        reported_at: Option<i64>,
        pub(crate) message_id: Option<String>,
        pub(crate) vod: Option<String>,
    }

    #[derive(Identifiable, AsChangeset)]
    #[table_name = "race_runs"]
    struct UpdateRaceRun {
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

    impl From<RaceRun> for UpdateRaceRun {
        fn from(rr: RaceRun) -> Self {
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
    impl RaceRun {
        pub(crate) async fn get_runs(
            race_id_: i32,
            conn: &mut SqliteConnection,
        ) -> Result<(RaceRun, RaceRun), String> {
            use crate::schema::race_runs::dsl::*;
            let mut runs: Vec<RaceRun> = race_runs
                .filter(race_id.eq(race_id_))
                .load(conn)
                .map_err(|e| e.to_string())?;
            if runs.len() == 2 {
                Ok((runs.pop().unwrap(), runs.pop().unwrap()))
            } else {
                Err("Did not find exactly 2 runs".to_string())
            }
        }
    }

    // instance
    impl RaceRun {
        pub(crate) fn racer_id(&self) -> Result<Id<UserMarker>, String> {
            self.racer_id
                .parse::<u64>()
                .map_err(|e| e.to_string())
                .map(Id::<UserMarker>::new)
                .map_err(|e| e.to_string())
        }

        pub(crate) fn is_finished(&self) -> bool {
            match self.state {
                RaceRunState::VOD_SUBMITTED => true,
                RaceRunState::FORFEIT => true,
                _ => false,
            }
        }

        pub(crate) fn filenames(&self) -> Result<Filenames, String> {
            Filenames::from_str(&self.filenames)
        }

        pub(crate) fn contact_succeeded(&mut self) {
            self.state = RaceRunState::CONTACTED;
        }

        pub(crate) fn get_message_id(&self) -> Option<Id<MessageMarker>> {
            self.message_id
                .as_ref()
                .and_then(|ms| Id::<MessageMarker>::from_str(&ms).ok())
        }

        pub(crate) fn get_started_at(&self) -> Option<NaiveDateTime> {
            self.run_started.map(timestamp_to_naivedatetime)
        }

        pub(crate) fn get_finished_at(&self) -> Option<NaiveDateTime> {
            self.run_finished.map(timestamp_to_naivedatetime)
        }

        pub(crate) fn get_time_to_finish(&self) -> Option<String> {
            time_delta_lifted(self.get_started_at(), self.get_finished_at())
                .map(format_duration_hms)
        }

        pub(crate) fn get_time_from_finish_to_report(&self) -> Option<String> {
            time_delta_lifted(self.get_finished_at(), self.get_reported_at())
                .map(format_duration_hms)
        }

        pub(crate) fn get_reported_at(&self) -> Option<NaiveDateTime> {
            self.reported_at
                .map(|t| NaiveDateTime::from_timestamp(t, 0))
        }

        pub(crate) fn start(&mut self) {
            self.state = RaceRunState::STARTED;
            self.run_started = Some(epoch_timestamp() as i64);
        }

        /// If the run is already finished this does *not* update it
        pub(crate) fn finish(&mut self) {
            self.state = RaceRunState::FINISHED;
            if self.run_finished.is_none() {
                self.run_finished = Some(epoch_timestamp() as i64);
            }
        }

        pub(crate) fn forfeit(&mut self) {
            self.state = RaceRunState::FORFEIT;
        }

        pub(crate) fn report_user_time(&mut self, user_time: String) {
            self.state = RaceRunState::TIME_SUBMITTED;
            self.reported_at = Some(epoch_timestamp().into());
            self.reported_run_time = Some(user_time);
        }

        pub(crate) fn set_vod(&mut self, vod: String) {
            self.state = RaceRunState::VOD_SUBMITTED;
            self.vod = Some(vod);
        }

        pub(crate) async fn save(&self, conn: &mut SqliteConnection) -> Result<(), String> {
            let update = UpdateRaceRun::from(self.clone());
            diesel::update(self)
                .set(update)
                .execute(conn)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }

        pub(crate) fn set_message_id(&mut self, message_id: u64) {
            self.message_id = Some(message_id.to_string());
        }

        pub(crate) async fn get_by_message_id(
            message_id: Id<MessageMarker>,
            conn: &mut SqliteConnection,
        ) -> Result<Self, String> {
            Self::search_by_message_id(message_id.clone(), conn)
                .await
                .and_then(|rr| rr.ok_or(format!("No RaceRun with Message ID {}", message_id.get())))
        }

        pub(crate) async fn search_by_message_id(
            message_id_: Id<MessageMarker>,
            conn: &mut SqliteConnection,
        ) -> Result<Option<Self>, String> {
            use crate::schema::race_runs::dsl::*;
            let mut runs: Vec<Self> = race_runs
                .filter(message_id.eq(message_id_.to_string()))
                .load(conn)
                .map_err(|e| e.to_string())?;
            Ok(runs.pop())
        }
    }

    // It is impossible in Rust to `impl Into<String> for Id<UserMarker>`
    // That means we can't insert a struct with an `Id<UserMarker>` in it, so we have to
    // convert it ourselves.

    #[derive(Insertable)]
    #[diesel(table_name=race_runs)]
    pub(crate) struct NewRaceRun {
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

    impl NewRaceRun {
        pub(crate) fn new(race_id: i32, racer_id: Id<UserMarker>) -> Self {
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
}
