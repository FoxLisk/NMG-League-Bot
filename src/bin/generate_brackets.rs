use nmg_league_bot::db::raw_diesel_cxn_from_env;
use nmg_league_bot::models::season::Season;

fn main() -> anyhow::Result<()> {
    dotenv::dotenv()?;
    let mut db = raw_diesel_cxn_from_env()?;
    let sn = Season::get_active_season(&mut db)?.unwrap();
    println!("{:?}", sn);
    let bs = sn.brackets(&mut db)?;
    println!("{:?}", bs);
    for mut b in bs {
        for _p in b.players(&mut db)? {}
        b.generate_pairings(&mut db)?;
        println!("Generated pairings for Bracket: {:?}", b);
    }
    Ok(())
}
