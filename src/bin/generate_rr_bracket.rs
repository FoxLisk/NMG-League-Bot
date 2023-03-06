use nmg_league_bot::db::{raw_diesel_cxn_from_env, run_migrations};
use nmg_league_bot::models::brackets::{BracketType, NewBracket};
use nmg_league_bot::models::player::NewPlayer;
use nmg_league_bot::models::player_bracket_entries::NewPlayerBracketEntry;
use nmg_league_bot::models::season::{ Season};

extern crate dotenv;

// Generates season, bracket, and 16 players, including 1 for me and 1 for my alt

#[tokio::main]
async fn main() {
    dotenv::dotenv().unwrap();
    let mut db = raw_diesel_cxn_from_env().unwrap();

    run_migrations(&mut db).unwrap();
    let mut players = vec![];
    for i in 0..4 {
        let p = NewPlayer::new(format!("rr_p{i}"), format!("rr_{i}"), None, None, true).save(&mut db).unwrap();
        players.push(p);
    }
    let sn = Season::get_active_season(&mut db).unwrap().unwrap();
    let nb = NewBracket::new(&sn, "RoundRobin Test", BracketType::RoundRobin).save(&mut db).unwrap();
    for p in players {
        NewPlayerBracketEntry::new(&nb, &p).save(&mut db).unwrap();
    }
}