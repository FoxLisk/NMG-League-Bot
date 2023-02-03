-- Your SQL goes here
ALTER TABLE seasons ADD COLUMN state TEXT NOT NULL DEFAULT '"Created"';

CREATE TABLE IF NOT EXISTS qualifier_submissions (
    id            INTEGER PRIMARY KEY NOT NULL,
    player_id     INTEGER NOT NULL,
    season_id     INTEGER NOT NULL,
    reported_time INTEGER NOT NULL,
    vod_link      TEXT NOT NULL,

    FOREIGN KEY(player_id) REFERENCES players(id),
    FOREIGN KEY(season_id) REFERENCES seasons(id)
)