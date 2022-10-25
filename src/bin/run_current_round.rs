use diesel::SqliteConnection;
use twilight_http::Client;
use nmg_league_bot::ChannelConfig;
use nmg_league_bot::constants::TOKEN_VAR;
use nmg_league_bot::db::raw_diesel_cxn_from_env;
use nmg_league_bot::models::bracket_races::{
    get_current_round_race_for_player, BracketRaceState, PlayerResult,
};
use nmg_league_bot::models::brackets::Bracket;
use nmg_league_bot::models::player::Player;
use nmg_league_bot::models::season::Season;
use nmg_league_bot::worker_funcs::trigger_race_finish;

fn hms_to_secs(h: u32, m: u32, s: u32) -> u32 {
    s + (m * 60) + (h * 60 * 60)
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().unwrap();
    let mut db = raw_diesel_cxn_from_env().unwrap();
    let client = Client::new(std::env::var(TOKEN_VAR).unwrap());

    let chans = ChannelConfig::new_from_env();
    let brackets = Season::get_active_season(&mut db)
        .unwrap()
        .unwrap()
        .brackets(&mut db)
        .unwrap();
    for bracket in brackets {
        run_bracket(bracket, &client, &chans, &mut db).await;
    }
}

async fn run_bracket(bracket: Bracket, client: &Client, chans: &ChannelConfig, conn: &mut SqliteConnection) {
    let round = bracket.current_round(conn).unwrap().unwrap();

    for player in bracket.players(conn).unwrap() {
        let mut race = get_current_round_race_for_player(&player, conn)
            .unwrap()
            .unwrap();
        println!("Race: {:?}", race);

        if race.state().unwrap() != BracketRaceState::Finished {
            let (p1r, p2r, other_guy) = if race.player_1_id == player.id {
                (
                    PlayerResult::Finish(hms_to_secs(1, 23, 45)),
                    PlayerResult::Finish(hms_to_secs(1, 24, 45)),
                    Player::get_by_id(race.player_2_id, conn).unwrap().unwrap()
                )
            } else {
                (
                    PlayerResult::Finish(hms_to_secs(1, 25, 18)),
                    PlayerResult::Finish(hms_to_secs(1, 34, 55)),
                    Player::get_by_id(race.player_1_id, conn).unwrap().unwrap()
                )
            };
            let mut info = race.info(conn).unwrap();
            info.racetime_gg_url = Some("https://racetime.gg/whatever-this-is-fake-testing-stuff".to_string());
            trigger_race_finish(
                race,
                &info,
                (&player, p1r),
                (&other_guy, p2r),
                conn,
                &client,
                &chans
            ).await.unwrap();
        } else {
            println!("Race already finished: {:?}", race);
        }
        println!("Round done yet? {:?}", round.all_races_finished(conn));
    }
}
