use nmg_league_bot::db::raw_diesel_cxn_from_env;
use nmg_league_bot::models::brackets::NewBracket;
use nmg_league_bot::models::player::NewPlayer;
use nmg_league_bot::models::player_bracket_entries::NewPlayerBracketEntry;
use nmg_league_bot::models::season::{NewSeason, Season};

extern crate dotenv;

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
    for i in 0..16 {
        let name = format!("player_{}", i);
        let discord_id = format!("{}", i);
        let racetime_username = format!("player_{}#{}", i, i);
        let np = NewPlayer::new(name, discord_id, racetime_username, true);
        let p = np.save(&mut db).unwrap();

        let entry = NewPlayerBracketEntry::new(&b, &p);
        let pbe = entry.save(&mut db).unwrap();
        println!("Player {:?}, pbe {:?}", p, pbe);
    }
}
