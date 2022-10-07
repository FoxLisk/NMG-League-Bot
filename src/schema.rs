// @generated automatically by Diesel CLI.

diesel::table! {
    bracket_races (id) {
        id -> Integer,
        bracket_id -> Integer,
        player_1_id -> Integer,
        player_2_id -> Integer,
        async_race_id -> Nullable<Integer>,
        scheduled_for -> Nullable<BigInt>,
        state -> Text,
        player_1_result -> Nullable<Text>,
        player_2_result -> Nullable<Text>,
        outcome -> Nullable<Text>,
    }
}

diesel::table! {
    brackets (id) {
        id -> Integer,
        name -> Text,
        season_id -> Integer,
        state -> Text,
        current_round -> Nullable<Integer>,
    }
}

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

diesel::table! {
    seasons (id) {
        id -> Integer,
        started -> BigInt,
        finished -> Nullable<BigInt>,
        format -> Text,
    }
}

diesel::table! {
    signups (id) {
        id -> Integer,
        player_id -> Integer,
        season_id -> Integer,
    }
}

diesel::joinable!(bracket_races -> brackets (bracket_id));
diesel::joinable!(bracket_races -> races (async_race_id));
diesel::joinable!(brackets -> seasons (season_id));
diesel::joinable!(race_runs -> races (race_id));
diesel::joinable!(signups -> players (player_id));
diesel::joinable!(signups -> seasons (season_id));

diesel::allow_tables_to_appear_in_same_query!(
    bracket_races,
    brackets,
    players,
    race_runs,
    races,
    seasons,
    signups,
);
