extern crate sqlx;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::fs::create_dir_all;
use std::path::Path;
use std::str::FromStr;

#[tokio::main]
async fn main() {

    let sqlite_db_path = match dotenv::var("DATABASE_URL") {
        Ok(v) => v,
        Err(_e) => {
            println!("cargo:warning=Missing DATABASE_URL environment variable");
            panic!();
        }
    };
    let path = sqlite_db_path
        .strip_prefix("sqlite://")
        .unwrap()
        .to_string();
    let parent_path = Path::new(&path).parent().unwrap();

    if let Err(e) = create_dir_all(&parent_path) {
        println!(
            "cargo:warning=Unable to create db directory {:?}: {}",
            parent_path, e
        );
        panic!();
    }

    let sco = SqliteConnectOptions::from_str(&*path)
        .unwrap()
        .create_if_missing(true)
        .foreign_keys(true);

    let pool = match SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(sco)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            println!("cargo:warning=Unable to create sqlite pool: {}", e);
            panic!()
        }
    };

    if let Err(e) = sqlx::migrate!().run(&pool).await {
        println!("cargo:warning=Error running migrations: {}", e);
        panic!();
    }
}
