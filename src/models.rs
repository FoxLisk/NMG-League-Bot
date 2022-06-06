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
    use serenity::model::id::UserId;
    use sqlx::SqlitePool;

    #[derive(sqlx::Type)]
    pub(crate) enum RaceState {
        CREATED,
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

    impl Race {
        async fn add_run(&self, racer_id: UserId, pool: &SqlitePool) -> Result<RaceRun, String> {
            let nrr = NewRaceRun::new(self.id, racer_id);
            nrr.save(pool).await
        }

        pub(crate) async fn select_racers(
            &self,
            racer_1: UserId,
            racer_2: UserId,
            pool: &SqlitePool,
        ) -> Result<(RaceRun, RaceRun), String> {
            if racer_1 == racer_2 {
                return Err("Racers must be different users!".to_string());
            }
            let r1 = self.add_run(racer_1, pool).await?;
            let r2 = self.add_run(racer_2, pool).await?;
            Ok((r1, r2))
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

    // -- represents a racer's participation in a race
    // CREATE TABLE IF NOT EXISTS race_runs (
    // id                INTEGER PRIMARY KEY NOT NULL,
    // race_id           INTEGER NOT NULL,
    // racer_id          INTEGER NOT NULL,
    // filenames         TEXT NOT NULL,
    // created           INTEGER NOT NULL,
    // state             TEXT NOT NULL,
    // run_started       INTEGER NULL,
    // run_finished      INTEGER NULL,
    // reported_run_time TEXT NULL,
    //
    // FOREIGN KEY(race_id) REFERENCES races(id)
    // );

    use serenity::model::id::{MessageId, UserId};

    use crate::models::epoch_timestamp;
    use sqlx::database::HasArguments;
    use sqlx::encode::IsNull;
    use sqlx::{Database, Encode, Sqlite, SqlitePool, Type};

    pub(crate) struct Filenames {
        pub(crate) one: char,
        pub(crate) three: [char; 3],
        pub(crate) four: [char; 4],
    }

    impl Filenames {
        fn new_random() -> Self {
            Self {
                one: 'a',
                three: ['a', 'b', 'c'],
                four: ['d', 'e', 'f', 'g'],
            }
        }

        fn from_str(value: &str) -> Result<Self, String> {
            let re = regex::Regex::new("([a-z]) ([a-z]{3}) ([a-z]{4})").unwrap();
            let caps = re
                .captures(value)
                .ok_or(format!("Invalid filenames field: {}", value))?;
            let one = caps
                .get(1)
                .ok_or(format!("Invalid filenames field: {}", value))?;
            let three_cap = caps
                .get(2)
                .ok_or(format!("Invalid filenames field: {}", value))?;
            let four_cap = caps
                .get(3)
                .ok_or(format!("Invalid filenames field: {}", value))?;

            let mut three_chars = three_cap.as_str().chars();
            let three: [char; 3] = [
                three_chars.next().unwrap(),
                three_chars.next().unwrap(),
                three_chars.next().unwrap(),
            ];

            let mut four_chars = four_cap.as_str().chars();
            let four: [char; 4] = [
                four_chars.next().unwrap(),
                four_chars.next().unwrap(),
                four_chars.next().unwrap(),
                four_chars.next().unwrap(),
            ];

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

    #[derive(sqlx::Type)]
    enum RaceRunState {
        CREATED,
        STARTED,
        FINISHED,
        TIME_SUBMITTED,
        VOD_SUBMITTED,
    }

    pub(crate) struct RaceRun {
        pub(crate) id: i64,
        race_id: i64,
        racer_id: String,
        filenames: String,
        created: u32,
        state: RaceRunState,
        message_id: Option<String>,
        run_started: Option<i64>,
        run_finished: Option<i64>,
        reported_run_time: Option<String>,
        reported_at: Option<u32>,
        vod: Option<String>,
    }

    impl RaceRun {
        pub(crate) fn racer_id(&self) -> UserId {
            UserId(self.racer_id.parse().unwrap())
        }

        pub(crate) fn filenames(&self) -> Result<Filenames, String> {
            Filenames::from_str(&self.filenames)
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

        pub(crate) fn set_message_id(&mut self, message_id: MessageId) {
            self.message_id = Some(message_id.to_string());
        }

        pub(crate) async fn get_by_message_id(
            message_id: &MessageId,
            pool: &SqlitePool,
        ) -> Result<Option<Self>, String> {
            let mid_str = message_id.to_string();
            sqlx::query_as!(
                Self,
                r#"SELECT id, race_id, racer_id, filenames,
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
        racer_id: UserId,
        filenames: Filenames,
        created: u32,
        state: RaceRunState,
    }

    impl NewRaceRun {
        pub(crate) fn new(race_id: i32, racer_id: UserId) -> Self {
            Self {
                race_id,
                racer_id,
                filenames: Filenames::new_random(),
                created: epoch_timestamp(),
                state: RaceRunState::CREATED,
            }
        }

        pub(crate) async fn save(self, pool: &SqlitePool) -> Result<RaceRun, String> {
            let id_str = self.racer_id.to_string();
            sqlx::query!(
                "INSERT INTO race_runs (race_id, racer_id, filenames, created, state)
                 VALUES(?, ?, ?, ?, ?);
                SELECT last_insert_rowid() as rowid;",
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
