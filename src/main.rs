use shutdown::Shutdown;
use twilight_model::channel::message::MessageFlags;

mod constants;
mod db;
mod discord;
mod models;
mod race_cron;
mod shutdown;
mod utils;
mod web;

extern crate oauth2;
extern crate rand;
extern crate regex;
extern crate rocket;
extern crate rocket_dyn_templates;
extern crate serenity;
extern crate sqlx;
extern crate tokio;
extern crate twilight_http;
extern crate twilight_model;
extern crate twilight_validate;
extern crate twilight_util;

use discord::Webhooks;

#[tokio::main]
async fn main() {
    dotenv::dotenv().unwrap();
    let webhooks = Webhooks::new().await.unwrap();

    let (shutdown_send, _) = tokio::sync::broadcast::channel::<Shutdown>(1);
    tokio::spawn(race_cron::cron(shutdown_send.subscribe(), webhooks));
    let (cache, guild_role_map) = discord::launch_discord_bot(shutdown_send.subscribe()).await;
    tokio::spawn(web::launch_website(
        cache,
        guild_role_map,
        shutdown_send.subscribe(),
    ));

    tokio::spawn(discord::bot_twilight::launch(shutdown_send.subscribe()));

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
