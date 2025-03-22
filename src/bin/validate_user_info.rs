use std::{collections::HashMap, time::Duration};

use clap::Parser;
use itertools::Itertools;
use nmg_league_bot::{config::CONFIG, models::player::Player, twitch_client::TwitchClientBundle};
use racetime_api::{client::RacetimeClient, endpoint::Query, endpoints::UserData, types::User};
use twitch_api::helix::users::GetUsersRequest;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    base_url: String,

    #[arg(long)]
    season_id: i32,
}

#[derive(Debug, serde::Deserialize)]
struct Qualifier {
    player_id: i32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv()?;
    let c = reqwest::Client::new();
    let racetime_client = RacetimeClient::new().expect("Unable to construct RacetimeClient");
    let twitch_client = TwitchClientBundle::new(
        CONFIG.twitch_client_id.clone(),
        CONFIG.twitch_client_secret.clone(),
    )
    .await
    .expect("Couldn't construct twitch client");

    let Args {
        base_url,
        season_id,
    } = Args::parse();

    // get list of qualifiers to find out what players to check
    let qual_resp = c
        .get(format!("{base_url}/api/v1/season/{season_id}/qualifiers"))
        .send()
        .await?;
    let parsed: Result<Vec<Qualifier>, String> = qual_resp.json().await?;
    let qualifiers = parsed.map_err(|e| anyhow::anyhow!(e))?;
    let player_ids = qualifiers
        .into_iter()
        .map(|q| q.player_id)
        .collect::<Vec<_>>();
    let query_string = player_ids
        .into_iter()
        .map(|id| format!("player_id={id}"))
        .join("&");

    // get players
    let players_resp = c
        .get(format!("{base_url}/api/v1/players?{query_string}"))
        .send()
        .await?;
    let parsed: Result<Vec<Player>, String> = players_resp.json().await?;
    let players = parsed.map_err(|e| anyhow::anyhow!(e))?;
    let mut twitches_to_check: HashMap<String, Player> = Default::default();

    let mut errors: HashMap<String, Vec<String>> = Default::default();
    let mut interval = tokio::time::interval(Duration::from_secs(1));

    // make sure they have data set. we also do racetime queries in this loop, because there's no bulk API
    // for that afaict. twitch searches are handled after

    // I wish we could *also* check that their discord is connected to their twitch account, but it seems
    // that getting discord user connections requires OAuth.
    for mut p in players {
        if let Some(ref id) = p.racetime_user_id {
            let ud = UserData::new(id.clone());
            let res: Result<User, _> = ud.query(&racetime_client).await;
            match res {
                Ok(u) => {
                    if !u.full_name.eq_ignore_ascii_case(
                        &p.racetime_username
                            .take()
                            .unwrap_or("<Missing RT.gg username>".to_string()),
                    ) {
                        errors.entry(p.name.clone()).or_default().push(format!(
                            "Player {}'s racetime_username is different than the username associated with their RT.gg ID: {u:?}",
                            p.name
                        ));
                    }

                    if p.twitch_user_login != u.twitch_name {
                        errors.entry(p.name.clone()).or_default().push(format!(
                            "Player {}'s racetime account is not associated with the twitch_user_login they have set", p.name
                        ));
                    }
                }
                Err(e) => println!("Error fetching RT.gg user data for player {}: {e}", p.name),
            }
        } else {
            errors
                .entry(p.name.clone())
                .or_default()
                .push(format!("Player {} has no racetime id", p.name));
        }

        if p.twitch_user_login.is_some() {
            twitches_to_check.insert(p.twitch_user_login.take().unwrap(), p);
        } else {
            errors
                .entry(p.name.clone())
                .or_default()
                .push(format!("Player {} has no twitch username", p.name));
        }
        // don't slam rtgg too much
        interval.tick().await;
    }
    // this stupid variable is just to get type inference to work
    let whatever = twitches_to_check
        .keys()
        .map(|s| s.as_str().into())
        .collect::<Vec<_>>();
    let getusers = GetUsersRequest::logins(whatever);
    let twitch_users = twitch_client.req_get(getusers).await?;
    for u in twitch_users.data {
        twitches_to_check.remove(&u.login.take());
    }
    for p in twitches_to_check.into_values() {
        errors.entry(p.name.clone()).or_default().push(format!(
            "Player {} has a twitch_user_login but that user was not found in the twitch API",
            p.name
        ));
    }

    for (name, errs) in errors {
        println!("Player {name}");
        for e in errs {
            println!("    {e}");
        }
    }

    Ok(())
}
