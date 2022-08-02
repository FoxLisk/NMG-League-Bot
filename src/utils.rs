use std::ffi::OsStr;
use std::str::FromStr;

pub fn format_secs(secs: u64) -> String {
    let mins = secs / 60;
    let hours = mins / 60;
    if hours > 0 {
        format!(
            "{hours}:{mins:02}:{secs:02}",
            hours = hours,
            mins = mins % 60,
            secs = secs % 60 % 60
        )
    } else {
        format!(
            "{mins:02}:{secs:02}",
            mins = mins % 60,
            secs = secs % 60 % 60
        )
    }
}

pub fn env_default<K: AsRef<OsStr>, D: FromStr>(key: K, default: D) -> D {
    if let Ok(v) = std::env::var(key) {
        match v.parse::<D>() {
            Ok(parsed) => parsed,
            Err(_e) => default,
        }
    } else {
        default
    }
}
