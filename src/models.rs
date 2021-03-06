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

pub(crate) mod race {
    use crate::models::race_run::{NewRaceRun, RaceRun};
    use crate::models::{epoch_timestamp, uuid_string};
    use serde::Serialize;
    use sqlx::SqlitePool;
    use twilight_model::id::marker::UserMarker;
    use twilight_model::id::Id;

    #[derive(sqlx::Type, Debug, Serialize)]
    pub(crate) enum RaceState {
        CREATED,
        FINISHED,
        ABANDONED,
    }

    pub(crate) struct NewRace {
        uuid: String,
        created: u32,
        state: RaceState,
    }

    pub(crate) struct Race {
        id: i32,
        pub(crate) uuid: String,
        created: u32,
        state: RaceState,
    }

    // statics
    impl Race {
        pub(crate) async fn active_races(pool: &SqlitePool) -> Result<Vec<Self>, String> {
            sqlx::query_as!(
                Self,
                r#"SELECT id as "id: _", uuid, created as "created: _", state as "state: _"
                FROM races
                WHERE state=?"#,
                RaceState::CREATED
            )
            .fetch_all(pool)
            .await
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

        pub(crate) async fn save(&self, pool: &SqlitePool) -> Result<(), String> {
            sqlx::query!(
                "UPDATE races
                SET state=?
                WHERE id=?",
                self.state,
                self.id
            )
            .execute(pool)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
        }

        async fn add_run(
            &self,
            racer_id: Id<UserMarker>,
            pool: &SqlitePool,
        ) -> Result<RaceRun, String> {
            let nrr = NewRaceRun::new(self.id, racer_id);
            nrr.save(pool).await
        }

        pub(crate) async fn select_racers(
            &self,
            racer_1: Id<UserMarker>,
            racer_2: Id<UserMarker>,
            pool: &SqlitePool,
        ) -> Result<(RaceRun, RaceRun), String> {
            if racer_1 == racer_2 {
                return Err("Racers must be different users!".to_string());
            }
            let r1 = self.add_run(racer_1, pool).await?;
            let r2 = self.add_run(racer_2, pool).await?;
            Ok((r1, r2))
        }

        pub(crate) async fn get_runs(
            &self,
            pool: &SqlitePool,
        ) -> Result<(RaceRun, RaceRun), String> {
            RaceRun::get_runs(self.id, pool).await
        }
    }

    impl NewRace {
        pub(crate) fn new() -> Self {
            Self {
                uuid: uuid_string(),
                created: epoch_timestamp(),
                state: RaceState::CREATED,
            }
        }

        pub(crate) async fn save(self, pool: &SqlitePool) -> Result<Race, String> {
            sqlx::query!(
                "INSERT INTO races (uuid, created, state) VALUES (?, ?, ?);\
                SELECT last_insert_rowid() as rowid;",
                self.uuid,
                self.created,
                self.state
            )
            .fetch_one(pool)
            .await
            .map(|row| Race {
                id: row.rowid,
                uuid: self.uuid,
                created: self.created,
                state: self.state,
            })
            .map_err(|e| e.to_string())
        }
    }
}

pub(crate) mod race_run {
    use crate::models::epoch_timestamp;
    use crate::models::uuid_string;
    use lazy_static::lazy_static;
    use rand::rngs::ThreadRng;
    use rand::{thread_rng, Rng};
    use serde::Serialize;
    use sqlx::database::HasArguments;
    use sqlx::encode::IsNull;
    use sqlx::{Database, Encode, Sqlite, SqlitePool, Type};
    use std::fmt::Formatter;
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

    impl Type<Sqlite> for Filenames {
        fn type_info() -> <Sqlite as Database>::TypeInfo {
            <&str as Type<Sqlite>>::type_info()
        }
    }

    impl<'q> Encode<'q, Sqlite> for Filenames {
        fn encode_by_ref(&self, buf: &mut <Sqlite as HasArguments<'q>>::ArgumentBuffer) -> IsNull {
            let mut s = String::with_capacity(10);
            s.push(self.one);
            s.push(' ');
            s.extend(self.three);
            s.push(' ');
            s.extend(self.four);
            Encode::encode_by_ref(&s, buf)
        }
    }

    #[derive(sqlx::Type, Debug, Serialize)]
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
    }

    impl RaceRunState {
        pub(crate) fn is_created(&self) -> bool {
            match self {
                Self::CREATED => true,
                _ => false,
            }
        }
    }

    pub(crate) struct RaceRun {
        pub(crate) id: i64,
        pub(crate) uuid: String,
        race_id: i64,
        racer_id: String,
        filenames: String,
        created: u32,
        pub(crate) state: RaceRunState,
        message_id: Option<String>,
        pub(crate) run_started: Option<i64>,
        pub(crate) run_finished: Option<i64>,
        pub(crate) reported_run_time: Option<String>,
        reported_at: Option<u32>,
        pub(crate) vod: Option<String>,
    }

    impl RaceRun {
        pub(crate) async fn get_runs(
            race_id: i32,
            pool: &SqlitePool,
        ) -> Result<(RaceRun, RaceRun), String> {
            let mut runs = sqlx::query_as!(
                RaceRun,
                r#"SELECT id, uuid, race_id, racer_id, filenames,
                created as "created: _",
                state as "state: _",
                run_started as "run_started: _",
                run_finished as "run_finished: _",
                reported_run_time,
                reported_at as "reported_at: _",
                vod,
                message_id
                 FROM race_runs WHERE race_id=?"#,
                race_id,
            )
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
            if runs.len() == 2 {
                Ok((runs.pop().unwrap(), runs.pop().unwrap()))
            } else {
                Err("Did not find exactly 2 runs".to_string())
            }
        }
    }

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
            self.reported_at = Some(epoch_timestamp());
            self.reported_run_time = Some(user_time);
        }

        pub(crate) fn set_vod(&mut self, vod: String) {
            self.state = RaceRunState::VOD_SUBMITTED;
            self.vod = Some(vod);
        }

        pub(crate) async fn save(&self, pool: &SqlitePool) -> Result<(), String> {
            let racer_id_str = self.racer_id.to_string();
            let mid_str = self.message_id.as_ref().map(|m| m.to_string());
            sqlx::query!(
                "UPDATE race_runs
                SET
                    race_id=?, racer_id=?, filenames=?, state=?, message_id=?,
                    run_started=?, run_finished=?, reported_at=?, reported_run_time=?,
                    vod=?
                 WHERE id=?;",
                self.race_id,
                racer_id_str,
                self.filenames,
                self.state,
                mid_str,
                self.run_started,
                self.run_finished,
                self.reported_at,
                self.reported_run_time,
                self.vod,
                self.id
            )
            .execute(pool)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
        }

        pub(crate) fn set_message_id(&mut self, message_id: u64) {
            self.message_id = Some(message_id.to_string());
        }

        pub(crate) async fn get_by_message_id(
            message_id: Id<MessageMarker>,
            pool: &SqlitePool,
        ) -> Result<Self, String> {
            Self::search_by_message_id(message_id.clone(), pool)
                .await
                .and_then(|rr| rr.ok_or(format!("No RaceRun with Message ID {}", message_id.get())))
        }

        pub(crate) async fn search_by_message_id(
            message_id: Id<MessageMarker>,
            pool: &SqlitePool,
        ) -> Result<Option<Self>, String> {
            let mid_str = message_id.to_string();
            sqlx::query_as!(
                Self,
                r#"SELECT
                id, uuid, race_id, racer_id, filenames,
                created as "created: _",
                state as "state: _",
                run_started as "run_started: _",
                run_finished as "run_finished: _",
                reported_run_time,
                reported_at as "reported_at: _",
                vod,
                message_id
                FROM race_runs
                WHERE message_id = ?"#,
                mid_str
            )
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())
        }
    }

    pub(crate) struct NewRaceRun {
        race_id: i32,
        uuid: String,
        racer_id: Id<UserMarker>,
        filenames: Filenames,
        created: u32,
        state: RaceRunState,
    }

    impl NewRaceRun {
        pub(crate) fn new(race_id: i32, racer_id: Id<UserMarker>) -> Self {
            Self {
                race_id,
                uuid: uuid_string(),
                racer_id,
                filenames: Filenames::new_random(),
                created: epoch_timestamp(),
                state: RaceRunState::CREATED,
            }
        }

        pub(crate) async fn save(self, pool: &SqlitePool) -> Result<RaceRun, String> {
            let id_str = self.racer_id.to_string();
            sqlx::query!(
                "INSERT INTO race_runs (uuid, race_id, racer_id, filenames, created, state)
                 VALUES(?, ?, ?, ?, ?, ?);
                SELECT last_insert_rowid() as rowid;",
                self.uuid,
                self.race_id,
                id_str,
                self.filenames,
                self.created,
                self.state
            )
            .fetch_one(pool)
            .await
            .map(|row| RaceRun {
                id: row.rowid as i64,
                uuid: self.uuid,
                race_id: self.race_id as i64,
                racer_id: self.racer_id.to_string(),
                filenames: self.filenames.to_str(),
                created: self.created,
                state: self.state,
                message_id: None,
                run_started: None,
                run_finished: None,
                reported_run_time: None,
                reported_at: None,
                vod: None,
            })
            .map_err(|e| e.to_string())
        }
    }
}
