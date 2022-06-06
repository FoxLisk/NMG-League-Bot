use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

pub(crate) async fn get_pool() -> Result<SqlitePool, sqlx::Error> {
    let sqlite_db_path = std::env::var("DATABASE_URL").unwrap();
    let sco = SqliteConnectOptions::from_str(&*sqlite_db_path)
        .unwrap()
        .create_if_missing(true)
        .foreign_keys(true);

    SqlitePoolOptions::new()
        .max_connections(12)
        .connect_with(sco)
        .await
}
