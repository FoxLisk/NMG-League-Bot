use nmg_league_bot::db::raw_diesel_cxn_from_env;
use nmg_league_bot::models::brackets::NewBracket;
use nmg_league_bot::models::player::NewPlayer;
use nmg_league_bot::models::player_bracket_entries::NewPlayerBracketEntry;
use nmg_league_bot::models::season::{NewSeason, Season};

extern crate dotenv;

// Generates season, bracket, and 16 players, including 1 for me and 1 for my alt

#[tokio::main]
async fn main() {
    dotenv::dotenv().unwrap();
    let mut db = raw_diesel_cxn_from_env().unwrap();
    if Season::get_active_season(&mut db).unwrap().is_some() {
        println!("Test data already generated");
        return;
    }

    let nsn = NewSeason::new("Test NMG");
    let sn = nsn.save(&mut db).unwrap();
    let nb = NewBracket::new(&sn, "Test Bracket 1");
    let b = nb.save(&mut db).unwrap();
    println!("season: {:?}, bracket: {:?}", sn, b);
    for np in get_players() {

        let p = np.save(&mut db).unwrap();

        let entry = NewPlayerBracketEntry::new(&b, &p);
        let pbe = entry.save(&mut db).unwrap();
        println!("Player {:?}, pbe {:?}", p, pbe);
    }
}

fn get_players() -> Vec<NewPlayer> {
    let mut players = vec![];
    for i in 0..14 {
        let name = format!("player_{}", i);
        let discord_id = format!("{}", i);
        let racetime_username = format!("player_{}#{}", i, i);
        let np = NewPlayer::new(name, discord_id, racetime_username, true);
        players.push(np);
    }

    let me = NewPlayer::new("FoxLisk", "255676979460702210", "FoxLisk#8582", true);
    let me_alt = NewPlayer::new("Me Test", "1031811909223206912", "NA#1234", true);
    players.push(me);
    players.push(me_alt);
    players
}