use nmg_league_bot::db::{raw_diesel_cxn_from_env, run_migrations};
use nmg_league_bot::models::brackets::{BracketType, NewBracket};

use nmg_league_bot::models::player::NewPlayer;
use nmg_league_bot::models::player_bracket_entries::NewPlayerBracketEntry;
use nmg_league_bot::models::season::Season;

extern crate dotenv;

// Generates a 4-player round robin bracket
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().unwrap();
    let mut db = raw_diesel_cxn_from_env().unwrap();

    run_migrations(&mut db).unwrap();
    let mut players = vec![];
    for i in 0..4 {
        let p = NewPlayer::new(format!("rr_p{i}"), format!("rr_{i}"), None, None, None)
            .save(&mut db)
            .unwrap();
        players.push(p);
    }
    let sn = Season::ensure_started_season(&mut db)?;
    let nb = NewBracket::new(&sn, "RoundRobin Test", BracketType::RoundRobin).save(&mut db)?;
    for p in players {
        NewPlayerBracketEntry::new(&nb, &p).save(&mut db)?;
    }
    Ok(())
}
