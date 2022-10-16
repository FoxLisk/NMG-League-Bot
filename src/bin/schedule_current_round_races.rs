use std::ops::Add;
use chrono::Duration;
use rand::{Rng, thread_rng};
use nmg_league_bot::db::raw_diesel_cxn_from_env;
use nmg_league_bot::models::bracket_races::{
    get_current_round_race_for_player, BracketRaceState, PlayerResult,
};
use nmg_league_bot::models::player::Player;
use nmg_league_bot::models::season::Season;


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

        let dur = Duration::hours(thread_rng().gen_range(24..172) );
        race.schedule(now + dur).ok();
        race.update(&mut db).unwrap();
    }

}
