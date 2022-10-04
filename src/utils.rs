use chrono::{Duration, NaiveDateTime};
use std::ffi::OsStr;
use std::str::FromStr;

pub fn format_hms(secs: u64) -> String {
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

pub fn format_duration_hms(d: Duration) -> String {
    format_hms(d.num_seconds() as u64)
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

pub fn timestamp_to_naivedatetime<T: Into<i64>>(ts: T) -> NaiveDateTime {
    NaiveDateTime::from_timestamp(ts.into(), 0)
}

pub fn time_delta_lifted(
    start: Option<NaiveDateTime>,
    end: Option<NaiveDateTime>,
) -> Option<Duration> {
    start.lift(end, |s, e| e.signed_duration_since(s))
}

pub fn lift_option<T, F, O>(o1: Option<T>, o2: Option<T>, f: F) -> Option<O>
where
    F: FnOnce(T, T) -> O,
{
    o1.and_then(|first| o2.map(|second| f(first, second)))
}

trait OptExt<T, O> {
    fn lift<F>(self, other: Self, f: F) -> Option<O>
    where
        F: FnOnce(T, T) -> O;
}

// N.B. I'd like to also make this work on &Option<T> but idk how to get the
// f(&T, &T) stuff to work correctly right now
// I'd also like to make this `trait Lift` and have it work on Result<> as well, but that
// runs into trouble b/c i can't figure out how to write `fn lift<..>(..) -> Self<T>`
// this is probably something to do with associated types but I don't feel like digging in today.
impl<T, O> OptExt<T, O> for Option<T> {
    fn lift<F>(self, other: Option<T>, f: F) -> Option<O>
    where
        F: FnOnce(T, T) -> O,
    {
        lift_option(self, other, f)
    }
}

pub trait ResultCollapse<T> {
    fn collapse(self) -> T;
}

impl<T> ResultCollapse<T> for Result<T, T> {
    fn collapse(self) -> T {
        match self {
            Ok(t) => {t}
            Err(e) => {e}
        }
    }
}
