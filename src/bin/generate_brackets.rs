use nmg_league_bot::db::raw_diesel_cxn_from_env;
use nmg_league_bot::models::season::Season;

fn main() {
    dotenv::dotenv().unwrap();
    let mut db = raw_diesel_cxn_from_env().unwrap();
    let sn = Season::get_active_season(&mut db).unwrap().unwrap();
    println!("{:?}", sn);
    let bs = sn.brackets(&mut db).unwrap();
    println!("{:?}", bs);
    for mut b in bs {
        println!("Bracket: {:?}", b);
        for p in b.players(&mut db).unwrap() {
            println!("  Player: {:?}", p);
        }
        b.generate_pairings(&mut db).unwrap();
    }
}
