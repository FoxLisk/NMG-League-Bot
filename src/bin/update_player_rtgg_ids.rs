use diesel::prelude::*;
use nmg_league_bot::db::raw_diesel_cxn_from_env;
use nmg_league_bot::models::player::Player;
use nmg_league_bot::schema::players;
use nmg_league_bot::utils::racetime_base_url;
use racetime_api::client::RacetimeClient;
use racetime_api::endpoint::Query;
use racetime_api::endpoints::{Leaderboards, UserSearch};
use racetime_api::types::{LeaderboardsResult, UserSearchResult};
use std::collections::{HashMap, HashSet};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv()?;
    // lets try getting everyone out of the leaderboard query first

    let mut db = raw_diesel_cxn_from_env()?;
    let all_players: Vec<Player> = players::table
        .filter(players::racetime_user_id.is_null())
        .filter(players::racetime_username.is_not_null())
        .load(&mut db)?;
    let mut needs_update: HashMap<String, Player> = all_players
        .into_iter()
        .map(|p| (p.racetime_username.clone().unwrap().to_lowercase(), p))
        .collect::<_>();
    if needs_update.is_empty() {
        println!("All players have their rtgg ids set already!");
        return Ok(());
    }
    println!(
        "Searching for rtgg ids for these users: {:?}",
        needs_update.keys()
    );
    let client = RacetimeClient::new_with_url(&racetime_base_url())?;
    for category in vec!["alttp", "alttpr"] {
        println!("Searching category {category}...");
        update_from_category(&mut needs_update, category, &mut db, &client).await?;
    }
    println!("Searching individually for these users: {needs_update:?}");

    let mut still_missing: HashSet<String> = Default::default();
    for player in needs_update.into_values() {
        let name = player.racetime_username.clone().unwrap();
        if !update_from_user_search(player, &mut db, &client).await? {
            still_missing.insert(name);
        }
    }
    if still_missing.is_empty() {
        println!("Found all users!");
    } else {
        println!("Unable to find the following players' rtgg ids: {still_missing:?}");
    }

    Ok(())
}

async fn update_from_category(
    needs_update: &mut HashMap<String, Player>,
    category_name: &str,
    db: &mut SqliteConnection,
    client: &RacetimeClient,
) -> anyhow::Result<()> {
    let lbs = Leaderboards::new(category_name);
    let res: LeaderboardsResult = lbs.query(&client).await?;

    for lb in res.leaderboards {
        for ranking in lb.rankings {
            let u = ranking.user;
            if let Some(mut p) = needs_update.remove(&u.full_name.to_lowercase()) {
                println!("Updating {p:?} - setting id to {}", u.id);
                p.racetime_user_id = Some(u.id);
                p.update(db)?;
            }
        }
    }

    Ok(())
}

async fn update_from_user_search(
    mut player: Player,
    db: &mut SqliteConnection,
    client: &RacetimeClient,
) -> anyhow::Result<bool> {
    let name = player.racetime_username.as_ref().unwrap();
    let us = UserSearch::from_term(name);
    let res: UserSearchResult = us.query(&client).await?;
    for u in res.results {
        if u.full_name.to_lowercase() == name.to_lowercase() {
            player.racetime_user_id = Some(u.id);
            player.update(db)?;
            return Ok(true);
        }
    }
    Ok(false)
}
