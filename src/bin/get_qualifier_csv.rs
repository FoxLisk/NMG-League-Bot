use clap::Parser;
use nmg_league_bot::utils::format_hms;
use serde::Deserialize;
use std::{collections::HashSet, error::Error};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Season number to fetch
    #[arg(short, long)]
    season: u32,
}

#[derive(Debug, Deserialize)]
struct Qualifier {
    player_name: String,
    time: u64,
    vod: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let url = format!(
        "https://nmg-league.foxlisk.com/api/v1/season/{}/qualifiers",
        args.season
    );
    let resp = reqwest::blocking::get(&url)?.json::<Result<Vec<Qualifier>, String>>()??;

    let mut wtr = csv::Writer::from_writer(std::io::stdout());
    wtr.write_record(&["player_name", "time", "vod"])?;
    let mut seen: HashSet<String> = Default::default();

    for qual in resp {
        if !seen.insert(qual.player_name.clone()) {
            continue;
        }
        let hms = format_hms(qual.time);
        wtr.write_record(&[qual.player_name, hms, qual.vod])?;
    }
    wtr.flush()?;
    Ok(())
}
