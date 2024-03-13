use nmg_league_bot::db::{raw_diesel_cxn_from_env, run_migrations};
use nmg_league_bot::models::brackets::{BracketType, NewBracket};

use nmg_league_bot::models::player::NewPlayer;
use nmg_league_bot::models::player_bracket_entries::NewPlayerBracketEntry;
use nmg_league_bot::models::season::Season;

extern crate dotenv;

// Generates a 4-player round robin bracket
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    #[cfg(feature = "development")]
    {
        create_the_bracket()?;
    }
    Ok(())
}

#[cfg(feature = "development")]
fn create_the_bracket() -> anyhow::Result<()> {
    dotenv::dotenv()?;
    let mut db = raw_diesel_cxn_from_env()?;

    run_migrations(&mut db)?;
    let mut players = Vec::with_capacity(4);
    players.push(
        NewPlayer::new(
            "foxlisk",
            "255676979460702210",
            Some("foxlisk#1234"),
            Some("foxlisk"),
        )
        .save(&mut db)?,
    );
    players.push(
        NewPlayer::new(
            "foxlisktest",
            "1031811909223206912",
            Some("foxlisktest#3456"),
            Some("bot_lisk"),
        )
        .save(&mut db)?,
    );

    for i in 0..(4 - players.len()) {
        let p = NewPlayer::new(format!("rr_p{i}"), format!("rr_{i}"), None, None).save(&mut db)?;
        players.push(p);
    }
    let sn = Season::ensure_started_season(&mut db)?;
    let nb = NewBracket::new(&sn, "RoundRobin Test", BracketType::RoundRobin).save(&mut db)?;
    for p in players {
        NewPlayerBracketEntry::new(&nb, &p).save(&mut db)?;
    }
    Ok(())
}
