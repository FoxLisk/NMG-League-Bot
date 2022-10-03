// @generated automatically by Diesel CLI.

diesel::table! {
    players (id) {
        id -> Integer,
        name -> Text,
        discord_id -> Text,
        racetime_username -> Text,
        restreams_ok -> Integer,
    }
}

diesel::table! {
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

diesel::table! {
    races (id) {
        id -> Integer,
        uuid -> Text,
        created -> BigInt,
        state -> Text,
    }
}

diesel::joinable!(race_runs -> races (race_id));

diesel::allow_tables_to_appear_in_same_query!(
    players,
    race_runs,
    races,
);
