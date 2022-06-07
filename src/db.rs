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
