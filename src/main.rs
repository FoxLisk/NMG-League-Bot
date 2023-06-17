use log::{debug, info, warn};
use once_cell::sync::Lazy;
use racetime_api::client::RacetimeClient;
use shutdown::Shutdown;
use std::path::Path;

mod discord;
mod schema;
mod shutdown;
mod web;
mod workers;

extern crate bb8;
extern crate chrono;
extern crate diesel;
extern crate diesel_enum_derive;
extern crate log4rs;
extern crate nmg_league_bot;
extern crate oauth2;
extern crate rand;
extern crate regex;
extern crate rocket;
extern crate rocket_dyn_templates;
extern crate tokio;
extern crate twilight_http;
extern crate twilight_mention;
extern crate twilight_model;
extern crate twilight_standby;
extern crate twilight_util;
extern crate twilight_validate;

use crate::discord::generate_invite_link;
use discord::Webhooks;
use nmg_league_bot::config::CONFIG;
use nmg_league_bot::db::raw_diesel_cxn_from_env;
use nmg_league_bot::db::run_migrations;
use nmg_league_bot::twitch_client::TwitchClientBundle;
use nmg_league_bot::utils::env_var;

#[tokio::main]
async fn main() {
    dotenv::dotenv().unwrap();
    // Config is lazy-loaded to make it a WORM type, but we need to make sure it loads correctly, so
    // we force it on startup.
    Lazy::force(&CONFIG);
    let log_config_path = env_var("LOG4RS_CONFIG_FILE");
    log4rs::init_file(Path::new(&log_config_path), Default::default())
        .expect("Couldn't initialize logging");
    println!("{:?}", generate_invite_link());

    let webhooks = Webhooks::new().await.expect("Unable to construct Webhooks");
    let (shutdown_send, _) = tokio::sync::broadcast::channel::<Shutdown>(1);
    let racetime_client = RacetimeClient::new().expect("Unable to construct RacetimeClient");

    let twitch_client = TwitchClientBundle::new(
        CONFIG.twitch_client_id.clone(),
        CONFIG.twitch_client_secret.clone(),
    )
    .await
    .expect("Couldn't construct twitch client");
    let state = discord::bot::launch(
        webhooks.clone(),
        racetime_client,
        twitch_client,
        shutdown_send.subscribe(),
    )
    .await;
    {
        let mut conn = raw_diesel_cxn_from_env().unwrap();
        let res = run_migrations(&mut conn).unwrap();
        debug!("Migrations: {:?}", res);
    }

    tokio::spawn(workers::async_race_worker::cron(
        shutdown_send.subscribe(),
        webhooks.clone(),
        state.clone(),
    ));

    let website_jh = tokio::spawn(web::launch_website(
        state.clone(),
        shutdown_send.subscribe(),
    ));

    tokio::spawn(workers::racetime_scanner_worker::cron(
        shutdown_send.subscribe(),
        state.clone(),
    ));

    tokio::spawn(workers::forfeit_own_races::cron(
        shutdown_send.subscribe(),
        state.clone(),
    ));

    drop(state);
    drop(webhooks);
    tokio::select! {
        anything = tokio::signal::ctrl_c() => {
            info!("Got ^C (ish): {anything:?}");
        }
        website = website_jh => {
            warn!("Website crashed or whatever: {website:?}");
        }
    }

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
    info!("Shutting down gracefully");
}
