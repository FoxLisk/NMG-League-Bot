use diesel::{Connection, SqliteConnection};
use nmg_league_bot::db::run_migrations;
use nmg_league_bot::models::brackets::{BracketType, NewBracket};
use nmg_league_bot::models::player::NewPlayer;
use nmg_league_bot::models::player_bracket_entries::NewPlayerBracketEntry;
use nmg_league_bot::models::season::NewSeason;

fn start_db() -> Result<SqliteConnection, anyhow::Error> {
    let mut db = SqliteConnection::establish(":memory:")?;
    run_migrations(&mut db)?;
    Ok(db)
}

#[test]
fn test() -> Result<(), anyhow::Error> {
    let mut db = start_db()?;
    let season = NewSeason::new("test_round_robin", "alttp", "Any% NMG").save(&mut db)?;
    let mut bracket =
        NewBracket::new(&season, "test_rr_bracket", BracketType::RoundRobin).save(&mut db)?;
    let mut players = vec![];
    for i in 0..4 {
        let player = NewPlayer::new(format!("player_{i}"), format!("{i}"), None, None, true)
            .save(&mut db)?;
        NewPlayerBracketEntry::new(&bracket, &player).save(&mut db)?;
        players.push(player);
    }
    bracket.generate_pairings(&mut db)?;
    let round = bracket
        .current_round(&mut db)?
        .ok_or(anyhow::anyhow!("No current round?!?"))?;
    let races = round.races(&mut db)?;
    assert_eq!(2, races.len());
    assert_eq!(
        vec![(1, 3), (2, 4)],
        races
            .iter()
            .map(|r| (r.player_1_id, r.player_2_id))
            .collect::<Vec<(i32, i32)>>()
    );

    Ok(())
}
