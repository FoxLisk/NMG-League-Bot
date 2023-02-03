use nmg_league_bot::db::raw_diesel_cxn_from_env;
use nmg_league_bot::models::player::NewPlayer;
use std::error::Error;
use std::fs::File;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn Error>> {
    dotenv::dotenv().unwrap();
    let mut args = std::env::args().skip(1);
    let filename = args.next().ok_or("Need filename!")?;
    if args.next().is_some() {
        return Err("One argument only!!!!".into());
    }
    let f = File::open(filename)?;
    let br = BufReader::new(f);
    let players: Vec<NewPlayer> = serde_json::from_reader(br)?;
    let mut db = raw_diesel_cxn_from_env()?;
    for player in players {
        match player.save(&mut db) {
            Ok(_) => {
                println!("Saved {}", player.name);
            }
            Err(e) => {
                println!("Error saving {}: {}", player.name, e);
            }
        }
    }
    Ok(())
}
