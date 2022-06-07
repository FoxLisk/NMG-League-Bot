use shutdown::Shutdown;

mod constants;
mod db;
mod models;
mod race_cron;
mod shutdown;
mod utils;
mod discord;

extern crate rand;
extern crate serenity;
extern crate sqlx;
extern crate tokio;

#[tokio::main]
async fn main() {
    dotenv::dotenv().unwrap();
    let (shutdown_send, _) = tokio::sync::broadcast::channel::<Shutdown>(1);
    tokio::spawn(race_cron::cron(shutdown_send.subscribe()));
    tokio::spawn(discord::launch_discord_bot(shutdown_send.subscribe()));

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
