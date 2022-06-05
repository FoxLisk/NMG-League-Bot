

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
    use serenity::model::id::UserId;
    use crate::models::{uuid_string, epoch_timestamp};
    use sqlx::{SqlitePool};
    use crate::models::race_run::{NewRaceRun, RaceRun};

    #[derive(sqlx::Type)]
    pub(crate) enum RaceState {
        CREATED
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

        pub(crate) async fn select_racers(&self, racer_1: UserId, racer_2: UserId, pool: &SqlitePool) -> Result<(RaceRun, RaceRun), String> {
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
                self.uuid, self.created, self.state
            ).fetch_one(pool)
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

    use serenity::model::id::UserId;

    use crate::models::epoch_timestamp;
    use sqlx::{SqlitePool, Encode, Sqlite, Type, Database};
    use sqlx::database::HasArguments;
    use sqlx::encode::IsNull;

    struct Filenames {
        one: char,
        three: [char; 3],
        four: [char; 4]
    }

    impl Filenames {
        fn new_random() -> Self {
            Self {
                one: 'a',
                three: ['a', 'b', 'c'],
                four: ['d', 'e', 'f', 'g'],
            }
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
            Encode::encode_by_ref(&s,buf, )
        }
    }

    #[derive(sqlx::Type)]
    enum RaceRunState {
        CREATED
    }
    
    pub(crate) struct RaceRun {
        id: i32,
        race_id: i32,
        pub(crate) racer_id: UserId,
        filenames: Filenames,
        created: u32,
        state: RaceRunState,
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
                racer_id: racer_id,
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
                self.race_id, id_str, self.filenames, self.created, self.state
            ).fetch_one(pool)
                .await
                .map(|row| RaceRun {
                    id: row.rowid,
                    race_id: self.race_id,
                    racer_id: self.racer_id,
                    filenames: self.filenames,
                    created: self.created,
                    state: self.state,
                })
                .map_err(|e| e.to_string())
        }
    }
}