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

    for i in 0..16 {
        let player = Player::get_by_discord_id(&format!("{}", i), &mut db)
            .unwrap()
            .unwrap();
        let mut race = get_current_round_race_for_player(&player, &mut db)
            .unwrap()
            .unwrap();
        println!("Race: {:?}", race);
        if race.state().unwrap() == BracketRaceState::New {
            if race.player_1_id == player.id {
                race.add_results(
                    Some(PlayerResult::Finish(123)),
                    Some(PlayerResult::Finish(234)),
                )
                .unwrap();
            } else {
                race.add_results(
                    Some(PlayerResult::Finish(234)),
                    Some(PlayerResult::Finish(123)),
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
