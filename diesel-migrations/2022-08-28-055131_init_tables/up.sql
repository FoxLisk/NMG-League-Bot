-- represents an asynchronous race
CREATE TABLE IF NOT EXISTS races (
    id           INTEGER PRIMARY KEY NOT NULL,
    uuid         TEXT UNIQUE NOT NULL,
    created      BIGINT NOT NULL,
    state        TEXT NOT NULL
);

-- represents a racer's participation in a race
CREATE TABLE IF NOT EXISTS race_runs (
    id                INTEGER PRIMARY KEY NOT NULL,
    uuid              TEXT UNIQUE NOT NULL,
    race_id           INTEGER NOT NULL,
    racer_id          TEXT NOT NULL,
    filenames         TEXT NOT NULL,
    created           BIGINT NOT NULL,
    state             TEXT NOT NULL,
    run_started       BIGINT NULL,
    run_finished      BIGINT NULL,
    reported_run_time TEXT NULL,
    reported_at       BIGINT NULL,
    message_id        TEXT NULL,
    vod               TEXT NULL,

    FOREIGN KEY(race_id) REFERENCES races(id)
);