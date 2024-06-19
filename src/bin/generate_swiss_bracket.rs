use nmg_league_bot::db::{raw_diesel_cxn_from_env, run_migrations};
use nmg_league_bot::models::brackets::{BracketType, NewBracket};
use nmg_league_bot::models::player::NewPlayer;
use nmg_league_bot::models::player_bracket_entries::NewPlayerBracketEntry;
use nmg_league_bot::models::season::{NewSeason, Season};
use nmg_league_bot::utils::uuid_string;

extern crate dotenv;

// Generates a 4-player round robin bracket
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv()?;
    let mut db = raw_diesel_cxn_from_env()?;

    run_migrations(&mut db)?;
    let mut players = vec![];
    let uuid = uuid_string();
    for i in 0..8 {
        let p = NewPlayer::new(
            format!("swiss_{uuid}_p{i}"),
            format!("swiss_{uuid}_{i}"),
            None,
            None,
        )
        .save(&mut db)?;
        players.push(p);
    }
    let sn = match Season::get_active_season(&mut db)? {
        Some(s) => s,
        None => {
            let s = NewSeason::new("my great format", "alttp", "Any% NMG", &mut db)?;
            s.save(&mut db)?
        }
    };
    let nb = NewBracket::new(&sn, "Swiss Test", BracketType::RoundRobin).save(&mut db)?;
    for p in players {
        NewPlayerBracketEntry::new(&nb, &p).save(&mut db).unwrap();
    }

    Ok(())
}
