table! {
    players (id) {
        id -> Integer,
        name -> Text,
        discord_id -> Text,
        restreams_ok -> Integer,
    }
}

table! {
    race_runs (id) {
        id -> Integer,
        uuid -> Text,
        race_id -> Integer,
        racer_id -> Text,
        filenames -> Text,
        created -> BigInt,
        state -> Text,
        run_started -> Nullable<BigInt>,
        run_finished -> Nullable<BigInt>,
        reported_run_time -> Nullable<Text>,
        reported_at -> Nullable<BigInt>,
        message_id -> Nullable<Text>,
        vod -> Nullable<Text>,
    }
}

table! {
    races (id) {
        id -> Integer,
        uuid -> Text,
        created -> BigInt,
        state -> Text,
    }
}

joinable!(race_runs -> races (race_id));

allow_tables_to_appear_in_same_query!(
    players,
    race_runs,
    races,
);
