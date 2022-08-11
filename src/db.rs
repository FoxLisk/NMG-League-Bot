use async_trait::async_trait;
use bb8::{ManageConnection, Pool};
use diesel::{Connection, ConnectionError, SqliteConnection};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;
use tokio::sync::Mutex;

use lazy_static::lazy_static;

lazy_static! {
    // was getting intermittent PoolTimedOut errors on `get_pool`, which i think this resolves?
    static ref DB_LOCK: Mutex<()> = Mutex::new(());
}

pub(crate) async fn get_pool() -> Result<SqlitePool, sqlx::Error> {
    let sqlite_db_path = std::env::var("DATABASE_URL").unwrap();
    let sco = SqliteConnectOptions::from_str(&*sqlite_db_path)
        .unwrap()
        .create_if_missing(true)
        .foreign_keys(true);

    let _lock = DB_LOCK.lock().await;

    SqlitePoolOptions::new()
        .max_connections(12)
        .connect_with(sco)
        .await
}

pub(crate) struct DieselConnectionManager {
    path: String,
}

impl DieselConnectionManager {
    fn new_from_env() -> Self {
        let sqlite_db_path = std::env::var("DATABASE_URL").unwrap();
        let path = if sqlite_db_path.starts_with("sqlite://") {
            sqlite_db_path
                .strip_prefix("sqlite://")
                .unwrap()
                .to_string()
        } else {
            sqlite_db_path
        };
        Self { path }
    }
}

#[async_trait]
impl ManageConnection for DieselConnectionManager {
    type Connection = SqliteConnection;
    type Error = ConnectionError;

    async fn connect(&self) -> Result<Self::Connection, Self::Error> {
        SqliteConnection::establish(&self.path)
    }

    // TODO: this is probably bad to leave unimplemented
    async fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        Ok(())
    }

    // TODO: this is probably bad to leave unimplemented
    fn has_broken(&self, conn: &mut Self::Connection) -> bool {
        false
    }
}

pub(crate) async fn get_diesel_pool() -> Pool<DieselConnectionManager> {
    let p = Pool::builder()
        .build(DieselConnectionManager::new_from_env())
        .await
        .unwrap();
    p
}
