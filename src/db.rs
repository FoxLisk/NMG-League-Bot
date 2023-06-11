use crate::utils::env_var;
use async_trait::async_trait;
use bb8::{ManageConnection, Pool};
use diesel::migration::MigrationVersion;
use diesel::{Connection, ConnectionError, SqliteConnection};
use diesel_migrations::{MigrationError, MigrationHarness};
use lazy_static::lazy_static;
use log::debug;
use std::error::Error;
use std::fs;
use std::path::Path;
use thiserror::Error;
use tokio::sync::{Mutex, MutexGuard};

lazy_static! {
    static ref DB_LOCK: Mutex<Option<Pool<DieselConnectionManager>>> = Mutex::new(None);
}
pub struct DieselConnectionManager {
    path: String,
}

fn munge_path(sqlite_db_path: String) -> String {
    if sqlite_db_path.starts_with("sqlite://") {
        sqlite_db_path
            .strip_prefix("sqlite://")
            .unwrap()
            .to_string()
    } else {
        sqlite_db_path
    }
}

pub fn raw_diesel_cxn_from_env() -> diesel::ConnectionResult<SqliteConnection> {
    let sqlite_db_path = env_var("DATABASE_URL");
    let path_s = munge_path(sqlite_db_path);
    let p = Path::new(&path_s);
    let dir = p.parent().unwrap();
    fs::create_dir_all(dir).unwrap();
    SqliteConnection::establish(&path_s)
}

#[derive(Error, Debug)]
pub enum RunMigrationsError {
    #[error("Migration error {0}")]
    MigrationsError(#[from] MigrationError),

    #[error("Database error {0}")]
    DatabaseError(#[from] Box<dyn Error + Send + Sync>),
}

pub fn run_migrations(
    conn: &mut SqliteConnection,
) -> Result<Vec<MigrationVersion>, RunMigrationsError> {
    let migrations = diesel_migrations::FileBasedMigrations::from_path("diesel-migrations")?;
    conn.run_pending_migrations(migrations).map_err(From::from)
}

impl DieselConnectionManager {
    fn new_from_env() -> Self {
        let sqlite_db_path = env_var("DATABASE_URL");
        let path = munge_path(sqlite_db_path);
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

pub async fn get_diesel_pool() -> Pool<DieselConnectionManager> {
    let mut something: MutexGuard<'_, Option<Pool<DieselConnectionManager>>> = DB_LOCK.lock().await;

    match &*something {
        Some(p) => {
            debug!("Returning existing diesel pool");
            p.clone()
        }
        None => {
            debug!("Generating a new diesel pool");
            let p = Pool::builder()
                .build(DieselConnectionManager::new_from_env())
                .await
                .unwrap();

            let out = p.clone();
            *something = Some(p);
            out
        }
    }
}
