use diesel::SqliteConnection;
use nmg_league_bot::db::{raw_diesel_cxn_from_env, run_migrations};
use nmg_league_bot::models::brackets::{BracketType, NewBracket};
use nmg_league_bot::models::player::NewPlayer;
use nmg_league_bot::models::player_bracket_entries::NewPlayerBracketEntry;
use nmg_league_bot::models::season::{NewSeason, Season};

extern crate dotenv;

// Generates season, bracket, and 16 players, including 1 for me and 1 for my alt

#[tokio::main]
async fn main() {
    dotenv::dotenv().unwrap();
    let mut db = raw_diesel_cxn_from_env().unwrap();

    run_migrations(&mut db).unwrap();

    if Season::get_active_season(&mut db).unwrap().is_some() {
        println!("Test data already generated");
        return;
    }

    let nsn = NewSeason::new("Test NMG", "alttp", "Any% NMG");
    let sn = nsn.save(&mut db).unwrap();

    generate_bracket(&sn, 1, BracketType::Swiss, &mut db).unwrap();
    generate_bracket(&sn, 2, BracketType::Swiss, &mut db).unwrap();
}

fn generate_bracket(
    season: &Season,
    id: i32,
    bt: BracketType,
    conn: &mut SqliteConnection,
) -> Result<(), diesel::result::Error> {
    let nb = NewBracket::new(season, format!("Test Bracket {id}"), bt);
    let b = nb.save(conn)?;
    println!("season: {:?}, bracket: {:?}", season, b);
    for np in get_players(16 * id, id == 1) {
        let p = np.save(conn)?;
        let entry = NewPlayerBracketEntry::new(&b, &p);
        let pbe = entry.save(conn)?;
        println!("Player {:?}, pbe {:?}", p, pbe);
    }
    Ok(())
}

fn get_players(start: i32, add_me: bool) -> Vec<NewPlayer> {
    let mut players = vec![];
    let end = if add_me { start + 14 } else { start + 16 };
    for i in start..end {
        let name = format!("player_{i}");
        let discord_id = format!("{i}");
        let racetime_username = format!("player_{i}#{i}");
        let twitch_id = format!("player_{i}_ttv");
        let np = NewPlayer::new(
            name,
            discord_id,
            Some(racetime_username),
            Some(twitch_id),
            true,
        );
        players.push(np);
    }

    if add_me {
        let me = NewPlayer::new(
            "FoxLisk",
            "255676979460702210",
            Some("FoxLisk#8582"),
            Some("foxlisk"),
            true,
        );
        let me_alt = NewPlayer::new(
            "Me Test",
            "1031811909223206912",
            Some("NA#1234"),
            Some("foxtest69"),
            true,
        );
        players.push(me);
        players.push(me_alt);
    }
    players
}
