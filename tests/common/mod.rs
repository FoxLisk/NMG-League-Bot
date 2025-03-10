use diesel::{Connection, SqliteConnection};
use nmg_league_bot::db::run_migrations;

pub fn start_db() -> Result<SqliteConnection, anyhow::Error> {
    let mut db = SqliteConnection::establish(":memory:")?;
    run_migrations(&mut db)?;
    Ok(db)
}
