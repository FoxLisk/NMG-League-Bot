use std::collections::{HashMap, HashSet};

use anyhow::anyhow;
use diesel::SqliteConnection;
use nmg_league_bot::{
    db::raw_diesel_cxn_from_env,
    models::{
        bracket_races::{BracketRaceState, NewBracketRace, Outcome, PlayerResult},
        bracket_rounds::NewBracketRound,
        brackets::{BracketType, NewBracket},
        player::{NewPlayer, Player},
        season::{NewSeason, Season, SeasonState},
    },
};
use rand::{thread_rng, Rng};

struct MakeBracket {
    new_bracket_name: &'static str,
    races: Vec<Vec<(&'static str, &'static str)>>,
}

struct MakeSeason {
    new_season: NewSeason,
    brackets: Vec<MakeBracket>,
}

fn main() -> anyhow::Result<()> {
    dotenv::dotenv()?;
    let dark_world = MakeBracket {
        new_bracket_name: "Dark World",
        races: vec![
            vec![
                ("foxlisk", "buane"),
                ("eriror", "mooglemod"),
                ("doomtap", "bydey"),
                ("fricker22", "benteezy"),
                ("thisisnotyoho", "vortexofdoom"),
                ("shkoople", "caznode"),
                ("chexhuman", "lanxion"),
                ("bambooshadow", "relkin"),
            ],
            vec![
                ("relkin", "benteezy"),
                ("buane", "lanxion"),
                ("caznode", "bydey"),
                ("mooglemod", "vortexofdoom"),
                ("doomtap", "shkoople"),
                ("eriror", "thisisnotyoho"),
                ("fricker22", "bambooshadow"),
                ("foxlisk", "chexhuman"),
            ],
            vec![
                ("relkin", "thisisnotyoho"),
                ("caznode", "mooglemod"),
                ("buane", "bambooshadow"),
                ("shkoople", "chexhuman"),
                ("bydey", "vortexofdoom"),
                ("benteezy", "lanxion"),
                ("doomtap", "fricker22"),
                ("eriror", "foxlisk"),
            ],
            vec![
                ("benteezy", "thisisnotyoho"),
                ("bydey", "bambooshadow"),
                ("chexhuman", "mooglemod"),
                ("lanxion", "vortexofdoom"),
                ("buane", "fricker22"),
                ("shkoople", "relkin"),
                ("foxlisk", "caznode"),
                ("eriror", "doomtap"),
            ],
        ],
    };

    let light_world = MakeBracket {
        new_bracket_name: "Light World",
        races: vec![
            vec![
                ("spleebie", "trinexx"),
                ("shadyforce", "mcmonkey"),
                ("cheamo", "tam"),
                ("yeroc", "rei"),
                ("parisianplayer", "daaanty"),
                ("john snuu", "coxla"),
                ("flipheal", "robjbeasley"),
                ("vextopher", "mraaronsnerd"),
            ],
            vec![
                ("yeroc", "robjbeasley"),
                ("tam", "mraaronsnerd"),
                ("mcmonkey", "coxla"),
                ("trinexx", "daaanty"),
                ("cheamo", "vextopher"),
                ("shadyforce", "john snuu"),
                ("spleebie", "parisianplayer"),
                ("flipheal", "rei"),
            ],
            vec![
                ("tam", "yeroc"),
                ("vextopher", "mcmonkey"),
                ("john snuu", "trinexx"),
                ("rei", "parisianplayer"),
                ("daaanty", "mraaronsnerd"),
                ("robjbeasley", "coxla"),
                ("spleebie", "cheamo"),
                ("shadyforce", "flipheal"),
            ],
            vec![
                ("shadyforce", "spleebie"),
                ("cheamo", "rei"),
                ("flipheal", "john snuu"),
                ("tam", "vextopher"),
                ("trinexx", "mcmonkey"),
                ("daaanty", "yeroc"),
                ("parisianplayer", "robjbeasley"),
                ("mraaronsnerd", "coxla"),
            ],
        ],
    };

    let s1 = MakeSeason {
        new_season: NewSeason {
            format: "Any% NMG".to_string(),
            started: 1654844400,
            state: serde_json::to_string(&SeasonState::Finished)?,
            rtgg_category_name: "alttp".to_string(),
            rtgg_goal_name: "Any% NMG".to_string(),
            ordinal: 1,
        },
        brackets: vec![dark_world, light_world],
    };

    let missing_players: HashMap<String, NewPlayer> = vec![
        (
            "fricker22",
            NewPlayer::new(
                "Fricker22",
                "335269633542062080",
                Some("Fricker22#5435"),
                Some("fricker22"),
                Some("5rNGD3DKwlB9blOy"),
            ),
        ),
        (
            "bambooshadow",
            NewPlayer::new(
                "BambooShadow",
                "167709096617705473",
                Some("BambooShadow#6580"),
                Some("bamboo_practices"),
                Some("N9rVpW9QYRWjq8Ll"),
            ),
        ),
        (
            "mooglemod",
            NewPlayer::new(
                "mooglemod",
                "267398828770983957",
                Some("mooglemod#8456"),
                Some("mooglemod"),
                Some("xldAMBl417BaOP57"),
            ),
        ),
        (
            "shkoople",
            NewPlayer::new(
                "shkoople",
                "98548933424332800",
                Some("shkoople#4144"),
                Some("shkoople"),
                Some("LNY0OkW1DZoKalP1"),
            ),
        ),
        (
            "buane",
            NewPlayer::new(
                "Buane",
                "124664588485656582",
                Some("Buane#5757"),
                Some("buane"),
                Some("41jgrbWPz3e7P5QE"),
            ),
        ),
        (
            "shadyforce",
            NewPlayer::new(
                "shadyforce",
                "379655195350794250",
                Some("shadyforce"),
                Some("shadyforcegames"),
                Some("NJrM6PoY5oRdm5v2"),
            ),
        ),
        (
            "mcmonkey",
            NewPlayer::new(
                "McMonkey",
                "178293242045923329",
                Some("McMonkey#7533"),
                Some("mcmonkey819"),
                Some("AEk8wpokxZW5KQyV"),
            ),
        ),
        (
            "parisianplayer",
            NewPlayer::new(
                "parisianplayer",
                "153404520259387393",
                Some("parisianplayer#2994"),
                Some("parisianplayer"),
                Some("8QGZrB2MP0WNgk4V"),
            ),
        ),
        (
            "daaanty",
            NewPlayer::new(
                "daaanty",
                "98625136076279808",
                Some("daaanty#0264"),
                Some("daaanty"),
                Some("yDMLq1oZDEBOeQG8"),
            ),
        ),
        (
            "mraaronsnerd",
            NewPlayer::new(
                "MrAaronSnerd",
                "724797547637506090",
                Some("MrAaronSnerd#8504"),
                Some("mraaronsnerd"),
                Some("MqzQPW4jNK31L2R5"),
            ),
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect::<_>();
    let mut db = raw_diesel_cxn_from_env()?;
    let orm_players = get_all_players(&mut db)?;
    if !validate_season(&s1, &missing_players, &orm_players)? {
        return Err(anyhow!("Invalid season (pre-player-creation)"));
    }
    for p in missing_players.into_values() {
        if let Err(e) = p.save(&mut db) {
            return Err(anyhow!("Error saving {p:?}: {e}"));
        }
    }

    let orm_players = get_all_players(&mut db)?;
    if !validate_season(&s1, &Default::default(), &orm_players)? {
        return Err(anyhow!("Invalid season (post-player-creation)"));
    }

    // for round in s1.into_iter() {}
    Ok(())
}

fn actually_make_season(
    season_data: MakeSeason,
    orm_players: HashMap<String, Player>,
    db: &mut SqliteConnection,
) -> anyhow::Result<()> {
    let season = match Season::get_by_ordinal(season_data.new_season.ordinal, db) {
        Ok(s) => s,
        Err(diesel::result::Error::NotFound) => season_data.new_season.save(db)?,
        Err(e) => {
            return Err(e)?;
        }
    };
    // let mut rng = thread_rng();
    for bracket_data in season_data.brackets {
        let nb = NewBracket::new(&season, bracket_data.new_bracket_name, BracketType::Swiss);
        let bracket = match nb.save(db) {
            Ok(b) => b,
            Err(e) => {
                println!(
                    "Warning: Error saving bracket {}: {e}",
                    bracket_data.new_bracket_name
                );
                continue;
            }
        };
        for (rn, races) in bracket_data.races.into_iter().enumerate() {
            let nr = NewBracketRound::new(&bracket, rn as i32);
            let round = match nr.save(db) {
                Ok(br) => br,
                Err(e) => {
                    println!(
                        "Error saving BracketRound {rn} for bracket {}",
                        bracket.name
                    );
                    break;
                }
            };
            for (p1_name, p2_name) in races {
                // this is just for optics to make it so not every race is won by player 1 teehee
                let p1 = match orm_players.get(p1_name) {
                    Some(p) => p,
                    None => {
                        println!("Error: Missing player {p1_name} from {}", bracket.name);
                        break;
                    }
                };
                let p2 = match orm_players.get(p2_name) {
                    Some(p) => p,
                    None => {
                        println!("Error: Missing player {p2_name} from {}", bracket.name);
                        break;
                    }
                };

                let nbr = NewBracketRace {
                    bracket_id: bracket.id,
                    round_id: round.id,
                    player_1_id: p1.id,
                    player_2_id: p2.id,
                    async_race_id: None,
                    state: serde_json::to_string(&BracketRaceState::Finished).unwrap(),
                    // NOTE: I am just putting in trash data for finish times
                    // At some point in the future, I could plausibly collect past results from
                    // racetime rooms
                    player_1_result: Some(
                        serde_json::to_string(&PlayerResult::Finish(60)).unwrap(),
                    ),
                    player_2_result: Some(
                        serde_json::to_string(&PlayerResult::Finish(120)).unwrap(),
                    ),
                    outcome: Some(serde_json::to_string(&Outcome::P1Win).unwrap()),
                };
                if let Err(e) = nbr.save(db) {
                    println!("Error saving race {p1_name} vs {p2_name}: {e}");
                }
            }
        }
    }
    Ok(())
}

fn get_all_players(db: &mut SqliteConnection) -> anyhow::Result<HashMap<String, Player>> {
    let players = Player::by_id(None, db)?
        .into_values()
        .map(|p| (p.name.to_lowercase(), p))
        .collect::<HashMap<String, Player>>();
    println!("called get_all_players: result is length {}", players.len());
    Ok(players)
}

fn validate_season(
    season: &MakeSeason,
    missing_players: &HashMap<String, NewPlayer>,
    orm_players: &HashMap<String, Player>,
) -> anyhow::Result<bool> {
    for bracket in &season.brackets {
        if !validate_bracket(&bracket.races, missing_players, orm_players)? {
            println!("Error validating {}", bracket.new_bracket_name);
            return Ok(false);
        }
    }

    Ok(true)
}

fn validate_bracket(
    season_data: &Vec<Vec<(&str, &str)>>,
    missing_players: &HashMap<String, NewPlayer>,
    orm_players: &HashMap<String, Player>,
) -> anyhow::Result<bool> {
    let expected_players: HashSet<String> = season_data[0]
        .iter()
        .map(|(a, b)| vec![a, b])
        .flatten()
        .map(|s| s.to_string())
        .collect::<_>();
    if expected_players.len() != 16 {
        println!("Wrong number of players! {}", expected_players.len());
        return Ok(false);
    }
    let mut seen_mus: HashSet<(&str, &str)> = Default::default();
    for (i, round) in season_data.iter().enumerate() {
        if round.len() != 8 {
            println!("Wrong number of matches in round {i}");
            return Ok(false);
        }
        for p in round.iter().map(|(a, b)| vec![a, b]).flatten() {
            let in_orm = orm_players.contains_key(*p);
            let in_missing = missing_players.contains_key(*p);
            if in_orm == in_missing {
                println!("Player `{p}' in bad state in round {i}: found in orm: {in_orm} - found in missing: {in_missing}.");
                // for (k, v) in orm_players.iter() {
                //     println!("key: {k}");
                //     // println!("player: {v:?}");
                // }
                return Ok(false);
            }
        }
        for (p1, p2) in round.iter() {
            for mu in [(*p1, *p2), (*p2, *p1)] {
                if !seen_mus.insert(mu) {
                    println!("Repeat matchup: {mu:?} in round {i}");
                    return Ok(false);
                }
            }
        }
    }

    Ok(true)
}
