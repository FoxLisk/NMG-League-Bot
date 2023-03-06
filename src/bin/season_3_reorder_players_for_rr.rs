use diesel::{SqliteConnection};
use diesel::prelude::*;
use nmg_league_bot::db::{raw_diesel_cxn_from_env, run_migrations};
use nmg_league_bot::models::player::Player;
use anyhow::{Result, anyhow};
use diesel::connection::SimpleConnection;
use nmg_league_bot::models::bracket_races::{PlayerResult};
use nmg_league_bot::models::brackets::{BracketType, NewBracket};
use nmg_league_bot::models::player_bracket_entries::NewPlayerBracketEntry;
use nmg_league_bot::models::season::Season;
use nmg_league_bot::utils::parse_hms;

fn main() -> Result<()> {
    std::fs::copy("db/sqlite.db3", "db/sqlite.db3.season_3_rr_backup")?;
    dotenv::dotenv().unwrap();
    let mut db = raw_diesel_cxn_from_env().unwrap();
    run_migrations(&mut db).unwrap();

    let windfox = get_player("WindFox470", &mut db)?;
    let norskmatty = get_player("norskmatty", &mut db)?;
    let narwhal = get_player("NiobiumNarwhal", &mut db)?;
    get_player("PowerToMario", &mut db)?;
    let mut sql_parts = vec![];
    sql_parts.push(build_recreate_sql(windfox));
    sql_parts.push(build_recreate_sql(norskmatty));
    sql_parts.push(build_recreate_sql(narwhal));
    db.batch_execute("PRAGMA foreign_keys = OFF;")?;

    let res = db.transaction(|c| do_stuff(c, sql_parts.join("\n")));
    db.batch_execute("PRAGMA foreign_keys = ON;")?;
    res
}

fn get_player(name: &str, conn: &mut SqliteConnection) -> Result<Player> {
    Player::get_by_name(name, conn)?.ok_or(anyhow!("Can't find {name}"))
}

fn do_stuff(conn: &mut SqliteConnection, create_sql: String)-> Result<()> {
    // first off recreate the players
    println!("Executing stuff");
    conn.batch_execute(&create_sql)?;
    // assuming that succeeds, create the bracket
    let season = Season::get_active_season(conn)?.ok_or(anyhow!("No active season"))?;
    let mut bracket = NewBracket::new(&season, "Hyrule Castle", BracketType::RoundRobin).save(conn)?;
    // we have to refetch them because they just got recreated with new IDs
    let windfox = get_player("WindFox470", conn)?;
    let norskmatty = get_player("norskmatty", conn)?;
    let narwhal = get_player("NiobiumNarwhal", conn)?;
    let ptm = get_player("PowerToMario", conn)?;
    for player in vec![&windfox, &norskmatty, &narwhal, &ptm] {
        NewPlayerBracketEntry::new(&bracket, player).save(conn)?;
    }
    bracket.generate_pairings(conn)?;
    let round = bracket.current_round(conn)?.ok_or(anyhow!("Failed to generate a first round"))?;
    let mut races_r1 = round.races(conn)?;
    let mut wf_nn = races_r1.pop().ok_or(anyhow!("Missing windfox vs niobium round 1"))?;
    if wf_nn.player_1_id != windfox.id {
        return Err(anyhow!("Unexpected ids"));
    }
    if wf_nn.player_2_id != narwhal.id {
        return Err(anyhow!("Unexpected ids"));
    }
    let wf_time = parse_hms("1:31:23").ok_or(anyhow!("failed time parsing"))?;
    wf_nn.add_results(Some(&PlayerResult::Finish(wf_time)), Some(&PlayerResult::Forfeit), false)?;
    wf_nn.update(conn)?;

    let mut ptm_norsk = races_r1.pop().ok_or(anyhow!("Missing norsk vs ptm round 1"))?;
    if ptm_norsk.player_1_id != ptm.id {
        return Err(anyhow!("Unexpected id for ptm_norsk (PTM)"));
    }
    if ptm_norsk.player_2_id != norskmatty.id {
        return Err(anyhow!("Unexpected id for ptm_norsk (norsk)"));
    }
    let norsk_time = parse_hms("1:30:58").ok_or(anyhow!("Failed time parsing"))?;
    ptm_norsk.add_results(Some(&PlayerResult::Forfeit), Some(&PlayerResult::Finish(norsk_time)), false)?;
    ptm_norsk.update(conn)?;

    bracket.generate_pairings(conn)?;
    let r2 = bracket.current_round(conn)?.ok_or(anyhow!("Failed to generate second round"))?;
    let mut races_r2 = r2.races(conn)?;
    let nn_nm = races_r2.pop().ok_or(anyhow!("Missing narwhal vs norskmatty round 2"))?;
    println!("{nn_nm:?}");
    if nn_nm.player_1_id != norskmatty.id {
        return Err(anyhow!("Unexpected id for nn_nm r2 (matty)"));
    }
    if nn_nm.player_2_id != narwhal.id {
        return Err(anyhow!("Unexpected id for nn_nm r2 (narwhal)"));
    }
    let wf_ptm = races_r2.pop().ok_or(anyhow!("Missing windfox vs ptm round 2"))?;
    if wf_ptm.player_1_id != windfox.id {
        return Err(anyhow!("Unexpected id for wf_ptm r2 (wf)"));
    }
    if wf_ptm.player_2_id != ptm.id {
        return Err(anyhow!("Unexpected id for wf_ptm r2 (ptm)"));
    }

    Ok(())
}



fn build_recreate_sql(p: Player) -> String {
    let Player {
        id, name, discord_id, racetime_username, twitch_user_login, restreams_ok
    } = p;
    let racetime= racetime_username.map(|s| format!(r#""{s}""#)).unwrap_or("NULL".to_string());
    let twitch= twitch_user_login.map(|s| format!(r#""{s}""#)).unwrap_or("NULL".to_string());
    format!(
r#"DELETE FROM players WHERE id = {id};
INSERT INTO players (name, discord_id, racetime_username, twitch_user_login, restreams_ok)
            VALUES ("{name}", "{discord_id}", {racetime}, {twitch}, {restreams_ok});
UPDATE qualifier_submissions SET player_id = last_insert_rowid() WHERE player_id = {id};
    "#)
}