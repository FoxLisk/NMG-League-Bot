use nmg_league_bot::{db::raw_diesel_cxn_from_env, models::season::Season};

/// this is a script to fix the data from prod, right before season 7,
/// after the patch to give seasons chronological (ordinal) information
/// and use the "format" column correctly
fn main() -> anyhow::Result<()> {
    dotenv::dotenv()?;
    let blargh = vec![
        // id, correct ordinal, correct format
        (1, 2, "Any% NMG"),
        (2, 3, "Any% NMG"),
        (3, 4, "Vanilla Preset Any% NMG"),
        (5, 5, "Any% NMG"),
        (6, 6, "Any% NMG"),
    ];
    let mut cxn = raw_diesel_cxn_from_env()?;
    for (id, ord, fmt) in blargh {
        let mut szn = Season::get_by_id(id, &mut cxn)?;
        szn.ordinal = ord;
        szn.format = fmt.to_string();
        szn.update(&mut cxn)?;
    }
    Ok(())
}
