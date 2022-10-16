use nmg_league_bot::db::raw_diesel_cxn_from_env;
use nmg_league_bot::models::bracket_races::{
    get_current_round_race_for_player, BracketRaceState, PlayerResult,
};
use nmg_league_bot::models::player::Player;
use nmg_league_bot::models::season::Season;

fn hms_to_secs(h: u32, m: u32, s: u32) -> u32 {
    s + (m * 60) + (h * 60 * 60)
}

fn main() {
    dotenv::dotenv().unwrap();
    let mut db = raw_diesel_cxn_from_env().unwrap();
    let bracket = Season::get_active_season(&mut db)
        .unwrap()
        .unwrap()
        .brackets(&mut db)
        .unwrap()
        .pop()
        .unwrap();
    let round = bracket
        .current_round(&mut db)
        .unwrap()
        .unwrap();

    for player in bracket.players(&mut db).unwrap() {

        let mut race = get_current_round_race_for_player(&player, &mut db)
            .unwrap()
            .unwrap();
        println!("Race: {:?}", race);
        if race.state().unwrap() == BracketRaceState::New {
            if race.player_1_id == player.id {
                race.add_results(
                    Some(PlayerResult::Finish(hms_to_secs(1, 23, 45))),
                    Some(PlayerResult::Finish(hms_to_secs(1, 24, 45))),
                )
                .unwrap();
            } else {
                race.add_results(
                    Some(PlayerResult::Finish(hms_to_secs(1, 25, 18))),
                    Some(PlayerResult::Finish(hms_to_secs(1, 34, 55))),
                )
                .unwrap();
            }
            race.update(&mut db).unwrap();
        } else {
            println!("Race already finished: {:?}", race);
        }
        println!("Round done yet? {:?}", round.all_races_finished(&mut db));
    }
}
