use std::collections::HashSet;

use anyhow::bail;
use clap::Parser;
use diesel::prelude::*;
use nmg_league_bot::{
    db::raw_diesel_cxn_from_env,
    models::player::NewPlayer,
    schema::{commentator_signups, players},
    utils::env_var,
};
use twilight_http::Client;
use twilight_model::id::{marker::UserMarker, Id};

#[derive(Debug, Parser)]
struct Args {
    /// Resolve and report the missing players without writing them.
    #[arg(long)]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv()?;
    let args = Args::parse();
    let mut db = raw_diesel_cxn_from_env()?;
    let client = Client::new(env_var("DISCORD_TOKEN"));

    let existing_discord_ids = players::table
        .select(players::discord_id)
        .load::<String>(&mut db)?
        .into_iter()
        .collect::<HashSet<_>>();
    let commentator_ids = commentator_signups::table
        .select(commentator_signups::discord_id)
        .distinct()
        .load::<String>(&mut db)?;
    let missing_ids = commentator_ids
        .into_iter()
        .filter(|id| !existing_discord_ids.contains(id))
        .collect::<Vec<_>>();

    println!(
        "Found {} commentator(s) without a player record.",
        missing_ids.len()
    );

    let mut hydrated = 0;
    let mut failures = Vec::new();
    for raw_id in missing_ids {
        let user_id = match raw_id.parse::<Id<UserMarker>>() {
            Ok(id) => id,
            Err(error) => {
                failures.push(format!("{raw_id}: invalid Discord ID: {error}"));
                continue;
            }
        };
        let user = match client.user(user_id).await {
            Ok(response) => match response.model().await {
                Ok(user) => user,
                Err(error) => {
                    failures.push(format!("{raw_id}: could not read Discord user: {error}"));
                    continue;
                }
            },
            Err(error) => {
                failures.push(format!("{raw_id}: could not fetch Discord user: {error}"));
                continue;
            }
        };
        let name = user.global_name.unwrap_or(user.name);
        println!("{raw_id}: {name}");
        if !args.dry_run {
            match NewPlayer::new(name, raw_id.clone(), None, None, None).save(&mut db) {
                Ok(_) => hydrated += 1,
                Err(error) => failures.push(format!("{raw_id}: could not create player: {error}")),
            }
        }
    }

    if args.dry_run {
        println!("Dry run complete; no player records were created.");
    } else {
        println!("Created {hydrated} player record(s).");
    }
    if !failures.is_empty() {
        for failure in &failures {
            eprintln!("{failure}");
        }
        bail!("{} commentator(s) could not be hydrated", failures.len());
    }
    Ok(())
}
