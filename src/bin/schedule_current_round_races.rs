use chrono::Duration;
use nmg_league_bot::db::raw_diesel_cxn_from_env;
use nmg_league_bot::models::season::Season;
use rand::{thread_rng, Rng};

fn main() {
    dotenv::dotenv().unwrap();
    let mut db = raw_diesel_cxn_from_env().unwrap();
    let round = Season::get_active_season(&mut db)
        .unwrap()
        .unwrap()
        .brackets(&mut db)
        .unwrap()
        .pop()
        .unwrap()
        .current_round(&mut db)
        .unwrap()
        .unwrap();

    let now = chrono::Local::now();
    for mut race in round.races(&mut db).unwrap() {
        let dur = Duration::hours(thread_rng().gen_range(24..172));
        race.schedule(&(now + dur), &mut db).ok();
        race.update(&mut db).unwrap();
    }
}
