use shutdown::Shutdown;

mod constants;
mod db;
mod discord;
mod schema;
mod shutdown;
mod web;
mod workers;

extern crate bb8;
extern crate chrono;
extern crate diesel;
extern crate diesel_enum_derive;
extern crate oauth2;
extern crate rand;
extern crate regex;
extern crate rocket;
extern crate rocket_dyn_templates;
extern crate tokio;
extern crate twilight_http;
extern crate twilight_model;
extern crate twilight_standby;
extern crate twilight_util;
extern crate twilight_validate;
extern crate twilight_mention;
extern crate nmg_league_bot;

use crate::db::raw_diesel_cxn_from_env;
use discord::Webhooks;
use nmg_league_bot::db::run_migrations;

#[tokio::main]
async fn main() {
    dotenv::dotenv().unwrap();

    let webhooks = Webhooks::new().await.unwrap();
    let (shutdown_send, _) = tokio::sync::broadcast::channel::<Shutdown>(1);

    let state = discord::bot::launch(webhooks.clone(), shutdown_send.subscribe()).await;

    {
        let mut conn = raw_diesel_cxn_from_env().unwrap();
        let res = run_migrations(&mut conn).unwrap();
        println!("Migrations: {:?}", res);
    }

    tokio::spawn(workers::async_race_worker::cron(
        shutdown_send.subscribe(),
        webhooks.clone(),
        state.clone(),
    ));

    tokio::spawn(web::launch_website(
        state.clone(),
        shutdown_send.subscribe(),
    ));

    tokio::spawn(workers::racetime_scanner_worker::cron(
        shutdown_send.subscribe(),
        state.clone()
    ));

    drop(state);
    drop(webhooks);
    tokio::signal::ctrl_c().await.ok();
    let (shutdown_signal_send, mut shutdown_signal_recv) = tokio::sync::mpsc::channel(1);
    // send a copy of an mpsc sender to each watcher of the shutdown thread...
    {
        shutdown_send
            .send(Shutdown {
                _handle: shutdown_signal_send.clone(),
            })
            .ok();
    }

    drop(shutdown_signal_send);
    shutdown_signal_recv.recv().await;
    println!("Shutting down gracefully");
}
