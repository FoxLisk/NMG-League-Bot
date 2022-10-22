// @generated automatically by Diesel CLI.

diesel::table! {
    bracket_race_infos (id) {
        id -> Integer,
        bracket_race_id -> Integer,
        scheduled_for -> Nullable<BigInt>,
        scheduled_event_id -> Nullable<Text>,
        commportunities_message_id -> Nullable<Text>,
    }
}

diesel::table! {
    bracket_races (id) {
        id -> Integer,
        bracket_id -> Integer,
        round_id -> Integer,
        player_1_id -> Integer,
        player_2_id -> Integer,
        async_race_id -> Nullable<Integer>,
        state -> Text,
        player_1_result -> Nullable<Text>,
        player_2_result -> Nullable<Text>,
        outcome -> Nullable<Text>,
    }
}

diesel::table! {
    bracket_rounds (id) {
        id -> Integer,
        round_num -> Integer,
        bracket_id -> Integer,
    }
}

diesel::table! {
    brackets (id) {
        id -> Integer,
        name -> Text,
        season_id -> Integer,
        state -> Text,
    }
}

diesel::table! {
    player_bracket_entry (id) {
        id -> Integer,
        bracket_id -> Integer,
        player_id -> Integer,
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

diesel::joinable!(bracket_race_infos -> bracket_races (bracket_race_id));
diesel::joinable!(bracket_races -> bracket_rounds (round_id));
diesel::joinable!(bracket_races -> brackets (bracket_id));
diesel::joinable!(bracket_races -> races (async_race_id));
diesel::joinable!(bracket_rounds -> brackets (bracket_id));
diesel::joinable!(brackets -> seasons (season_id));
diesel::joinable!(player_bracket_entry -> brackets (bracket_id));
diesel::joinable!(player_bracket_entry -> players (player_id));
diesel::joinable!(race_runs -> races (race_id));

diesel::allow_tables_to_appear_in_same_query!(
    bracket_race_infos,
    bracket_races,
    bracket_rounds,
    brackets,
    player_bracket_entry,
    players,
    race_runs,
    races,
    seasons,
);
