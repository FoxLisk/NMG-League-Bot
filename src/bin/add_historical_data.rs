use std::collections::{HashMap, HashSet};

use anyhow::anyhow;
use diesel::SqliteConnection;
use itertools::Itertools as _;
use nmg_league_bot::{
    db::raw_diesel_cxn_from_env,
    models::{
        bracket_races::{NewBracketRace, Outcome, PlayerResult},
        bracket_rounds::NewBracketRound,
        brackets::{BracketType, NewBracket},
        player::{NewPlayer, Player},
        player_bracket_entries::NewPlayerBracketEntry,
        season::{NewSeason, Season, SeasonState},
    },
    utils::parse_race_result,
    BracketRaceState,
};
use rand::{thread_rng, Rng};

struct RaceResult {
    player_1_name: String,
    player_2_name: String,
    player_1_result: PlayerResult,
    player_2_result: PlayerResult,
    outcome: Outcome,
}

impl From<(&'static str, &'static str)> for RaceResult {
    fn from((p1, p2): (&'static str, &'static str)) -> Self {
        Self::from((p1, "0:01:00", p2, "0:02:00"))
    }
}

impl From<(&'static str, &'static str, &'static str, &'static str)> for RaceResult {
    fn from(
        (p1, p1_time, p2, p2_time): (&'static str, &'static str, &'static str, &'static str),
    ) -> Self {
        let (final_p1, final_p1_time, final_p2, final_p2_time) = if thread_rng().gen_bool(0.5) {
            (p1, p1_time, p2, p2_time)
        } else {
            (p2, p2_time, p1, p1_time)
        };

        let p1_result =
            parse_race_result(final_p1_time).expect(&format!("failed to parse {final_p1_time})"));
        let p2_result =
            parse_race_result(final_p2_time).expect(&format!("failed to parse {final_p2_time})"));
        let outcome: Outcome = From::from((&p1_result, &p2_result));
        Self {
            player_1_name: final_p1.to_string(),
            player_2_name: final_p2.to_string(),
            player_1_result: p1_result,
            player_2_result: p2_result,
            outcome,
        }
    }
}

struct MakeBracket {
    new_bracket_name: &'static str,
    backfill_note: String,
    races: Vec<Vec<RaceResult>>,
    bracket_type: BracketType,
}

fn main() -> anyhow::Result<()> {
    dotenv::dotenv()?;
    let no_times_backfill_note = "This is a backfill of a historical bracket that was run off-site. The times included here are placeholders. They may be updated at some future time.".to_string();
    let dark_world = MakeBracket {
        new_bracket_name: "Dark World",
        backfill_note: no_times_backfill_note.clone(),
        bracket_type: BracketType::Swiss,
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
        ]
        .into_iter()
        .map(|i| i.into_iter().map(From::from).collect())
        .collect(),
    };

    let light_world = MakeBracket {
        new_bracket_name: "Light World",
        backfill_note: no_times_backfill_note.clone(),

        bracket_type: BracketType::Swiss,
        races: vec![
            vec![
                ("spleebie", "trinexx"),
                ("shadyforce", "mcmonkey"),
                ("cheamo", "tam"),
                ("yeroc", "rei"),
                ("parisianplayer", "daaanty"),
                ("john snuu", "coxla"),
                ("flipheal", "robjbeasley"),
                ("vextopher", "aurorasnerd"),
            ],
            vec![
                ("yeroc", "robjbeasley"),
                ("tam", "aurorasnerd"),
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
                ("daaanty", "aurorasnerd"),
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
                ("aurorasnerd", "coxla"),
            ],
        ]
        .into_iter()
        .map(|i| i.into_iter().map(From::from).collect())
        .collect(),
    };

    let ns1 = NewSeason {
        format: "Any% NMG".to_string(),
        started: 1654844400,
        state: serde_json::to_string(&SeasonState::Finished)?,
        rtgg_category_name: "alttp".to_string(),
        rtgg_goal_name: "Any% NMG".to_string(),
        ordinal: 1,
    };

    let s2_rain_state = MakeBracket {
        new_bracket_name: "Rain State",
        backfill_note: "This is a backfill of a historical bracket that was run off-site."
            .to_string(),

        bracket_type: BracketType::RoundRobin,
        races: vec![vec![
            ("kaederukawa", "1:33:33", "windfox470", "1:34:01"),
            (
                "defi_smiles",
                "1:40:23",
                "jesse (radicalsniper99)",
                "1:32:16",
            ),
            ("kaederukawa", "1:35:16", "defi_smiles", "forfeit"),
            (
                "windfox470",
                "forfeit",
                "jesse (radicalsniper99)",
                "1:32:05",
            ),
            (
                "kaederukawa",
                "1:31:46",
                "jesse (radicalsniper99)",
                "1:32:25",
            ),
            ("windfox470", "forfeit", "defi_smiles", "forfeit"),
        ]
        .into_iter()
        .map(From::from)
        .collect()],
    };

    let s4_hc = MakeBracket {
        new_bracket_name: "Hyrule Castle",
        backfill_note: no_times_backfill_note.clone(),

        bracket_type: BracketType::RoundRobin,
        races: vec![vec![
            // manually entering the fake times so that i can put soyhr as 'forfeit'... sigh
            ("redkitsune", "0:01:00", "joshbittner", "0:02:00"),
            ("niobiumnarwhal", "0:02:00", "seanrhapsody", "0:01:00"),
            ("redkitsune", "0:01:00", "niobiumnarwhal", "0:02:00"),
            ("joshbittner", "0:01:00", "soyhr", "forfeit"),
            ("niobiumnarwhal", "0:01:00", "soyhr", "forfeit"),
            ("redkitsune", "0:01:00", "seanrhapsody", "0:02:00"),
            ("seanrhapsody", "0:01:00", "soyhr", "forfeit"),
            ("joshbittner", "0:01:00", "niobiumnarwhal", "0:02:00"),
            ("seanrhapsody", "0:01:00", "joshbittner", "0:02:00"),
            ("redkitsune", "0:01:00", "soyhr", "forfeit"),
        ]
        .into_iter()
        .map(From::from)
        .collect()],
    };

    let missing_players: HashMap<String, NewPlayer> = vec![
        // (
        //     "fricker22",
        //     NewPlayer::new(
        //         "Fricker22",
        //         "335269633542062080",
        //         Some("Fricker22#5435"),
        //         Some("fricker22"),
        //         Some("5rNGD3DKwlB9blOy"),
        //     ),
        // ),
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
        // (
        //     "shkoople",
        //     NewPlayer::new(
        //         "shkoople",
        //         "98548933424332800",
        //         Some("shkoople#4144"),
        //         Some("shkoople"),
        //         Some("LNY0OkW1DZoKalP1"),
        //     ),
        // ),
        // (
        //     "buane",
        //     NewPlayer::new(
        //         "Buane",
        //         "124664588485656582",
        //         Some("Buane#5757"),
        //         Some("buane"),
        //         Some("41jgrbWPz3e7P5QE"),
        //     ),
        // ),
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
            "aurorasnerd",
            NewPlayer::new(
                "AuroraSnerd",
                "724797547637506090",
                Some("AuroraSnerd#0011"),
                Some("aurorasnerd"),
                Some("MqzQPW4jNK31L2R5"),
            ),
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect::<_>();
    let mut db = raw_diesel_cxn_from_env()?;

    let orm_players = get_all_players(&mut db)?;
    validate_players(&missing_players, &orm_players)?;

    for p in missing_players.into_values() {
        if let Err(e) = p.save(&mut db) {
            return Err(anyhow!("Error saving {p:?}: {e}"));
        }
    }

    let orm_players = get_all_players(&mut db)?;
    let s2 = Season::get_by_ordinal(2, &mut db)?;
    let s4 = Season::get_by_ordinal(4, &mut db)?;

    let s1 = ns1.save(&mut db)?;
    make_bracket(&s1, dark_world, &orm_players, &mut db)?;
    make_bracket(&s1, light_world, &orm_players, &mut db)?;

    make_bracket(&s2, s2_rain_state, &orm_players, &mut db)?;
    make_bracket(&s4, s4_hc, &orm_players, &mut db)?;

    Ok(())
}

fn validate_players(
    missing_players: &HashMap<String, NewPlayer>,
    orm_players: &HashMap<String, Player>,
) -> anyhow::Result<()> {
    let existing_discord_ids = orm_players
        .values()
        .map(|v| v.discord_id.clone())
        .collect::<HashSet<_>>();
    let mut not_missing_players = vec![];
    for p in missing_players.values() {
        if existing_discord_ids.contains(&p.discord_id) {
            not_missing_players.push(p.name.clone());
        }
    }
    if !not_missing_players.is_empty() {
        return Err(anyhow!(
            "The following players are already in the database: {}",
            not_missing_players.iter().join(", ")
        ));
    }
    Ok(())
}

fn make_bracket(
    season: &Season,
    bracket_data: MakeBracket,
    orm_players: &HashMap<String, Player>,
    db: &mut SqliteConnection,
) -> anyhow::Result<()> {
    let nb = NewBracket::new(
        &season,
        bracket_data.new_bracket_name,
        bracket_data.bracket_type,
    );
    let mut bracket = match nb.save(db) {
        Ok(b) => b,
        Err(e) => {
            return Err(anyhow!(
                "Warning: Error saving bracket {}: {e}",
                bracket_data.new_bracket_name
            ));
        }
    };
    let finished = bracket.finish(db)?;
    if !finished {
        return Err(anyhow!(
            "Error: Bracket {} was not finished after saving",
            bracket_data.new_bracket_name
        ));
    }
    bracket.backfill_note = Some(bracket_data.backfill_note.clone());
    bracket.update(db)?;

    let mut players_in_bracket: HashSet<&Player> = Default::default();

    for (mut rn, races) in bracket_data.races.into_iter().enumerate() {
        rn += 1; // round 0 lol
        let nr = NewBracketRound::new(&bracket, rn as i32);
        let round = match nr.save(db) {
            Ok(br) => br,
            Err(e) => {
                return Err(anyhow!(
                    "Error saving BracketRound {rn} for bracket {}: {e}",
                    bracket.name
                ));
            }
        };
        for race_result in races {
            let p1_name = &race_result.player_1_name;
            let p2_name = &race_result.player_2_name;
            let p1 = match orm_players.get(p1_name) {
                Some(p) => p,
                None => {
                    return Err(anyhow!("Missing player {p1_name} from {}", bracket.name));
                }
            };
            players_in_bracket.insert(p1);
            let p2 = match orm_players.get(p2_name) {
                Some(p) => p,
                None => {
                    return Err(anyhow!("Missing player {p2_name} from {}", bracket.name));
                }
            };
            players_in_bracket.insert(p2);

            let nbr = NewBracketRace {
                bracket_id: bracket.id,
                round_id: round.id,
                player_1_id: p1.id,
                player_2_id: p2.id,
                async_race_id: None,
                state: serde_json::to_string(&BracketRaceState::Finished).unwrap(),

                player_1_result: Some(serde_json::to_string(&race_result.player_1_result).unwrap()),
                player_2_result: Some(serde_json::to_string(&race_result.player_2_result).unwrap()),
                outcome: Some(serde_json::to_string(&race_result.outcome).unwrap()),
            };
            if let Err(e) = nbr.save(db) {
                return Err(anyhow!("Error saving race {p1_name} vs {p2_name}: {e}"));
            }
        }
    }
    for player in players_in_bracket {
        let npbe = NewPlayerBracketEntry::new(&bracket, player);
        npbe.save(db)?;
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
