use diesel::SqliteConnection;
use nmg_league_bot::db::{raw_diesel_cxn_from_env, run_migrations};
use nmg_league_bot::models::brackets::{BracketType, NewBracket};
use nmg_league_bot::models::player::NewPlayer;
use nmg_league_bot::models::player_bracket_entries::NewPlayerBracketEntry;
use nmg_league_bot::models::season::{NewSeason, Season, SeasonState};

extern crate dotenv;

const PLAYERS_PER_BRACKET: i32 = 8;

// Generates season, bracket, and 16 players, including 1 for me and 1 for my alt

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().unwrap();
    let mut db = raw_diesel_cxn_from_env()?;

    run_migrations(&mut db)?;

    if Season::get_active_season(&mut db)?.is_some() {
        return Err(anyhow::anyhow!("Test data already generated"));
    }

    let nsn = NewSeason::new("Test NMG", "alttp", "Any% NMG", &mut db)?;
    let mut sn = nsn.save(&mut db).unwrap();
    sn.set_state(SeasonState::QualifiersOpen, &mut db)?;
    sn.set_state(SeasonState::QualifiersClosed, &mut db)?;
    sn.set_state(SeasonState::Started, &mut db)?;
    sn.update(&mut db)?;

    generate_bracket(&sn, 1, BracketType::Swiss, &mut db)?;
    Ok(())
}

fn generate_bracket(
    season: &Season,
    id: i32,
    bt: BracketType,
    conn: &mut SqliteConnection,
) -> Result<(), diesel::result::Error> {
    let nb = NewBracket::new(season, format!("Test Bracket {id}"), bt);
    let b = nb.save(conn)?;
    println!("season: {season:?}, bracket: {b:?}");
    for np in get_players(PLAYERS_PER_BRACKET * id, id == 1) {
        let p = np.save(conn)?;
        let entry = NewPlayerBracketEntry::new(&b, &p);
        let pbe = entry.save(conn)?;
        println!("Player {p:?}, pbe {pbe:?}");
    }
    Ok(())
}

fn get_players(start: i32, add_me: bool) -> Vec<NewPlayer> {
    let mut players = vec![];
    let end = if add_me {
        start + PLAYERS_PER_BRACKET - 2
    } else {
        start + PLAYERS_PER_BRACKET
    };
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
            None,
        );
        players.push(np);
    }

    if add_me {
        let me = NewPlayer::new(
            "FoxLisk",
            "255676979460702210",
            Some("FoxLisk#8582"),
            Some("foxlisk"),
            None,
        );
        let me_alt = NewPlayer::new(
            "Me Test",
            "1031811909223206912",
            Some("NA#1234"),
            Some("foxtest69"),
            None,
        );
        players.push(me);
        players.push(me_alt);
    }
    players
}
