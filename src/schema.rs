use diesel::{allow_tables_to_appear_in_same_query, joinable, table};
table! {
    _sqlx_migrations (version) {
        version -> Nullable<BigInt>,
        description -> Text,
        installed_on -> Timestamp,
        success -> Bool,
        checksum -> Binary,
        execution_time -> BigInt,
    }
}

table! {
    race_runs (id) {
        id -> BigInt,
        uuid -> Text,
        race_id -> BigInt,
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
        id -> BigInt,
        uuid -> Text,
        created -> BigInt,
        state -> Text,
    }
}

joinable!(race_runs -> races (race_id));

allow_tables_to_appear_in_same_query!(
    _sqlx_migrations,
    race_runs,
    races,
);
