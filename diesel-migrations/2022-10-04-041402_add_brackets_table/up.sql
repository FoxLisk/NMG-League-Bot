-- Your SQL goes here
CREATE TABLE if not exists brackets (
    id           INTEGER PRIMARY KEY NOT NULL,
    name         TEXT NOT NULL,
    season_id    INTEGER NOT NULL,
    state        TEXT NOT NULL,
    current_round INTEGER NULL,

    FOREIGN KEY(season_id) REFERENCES seasons(id)
);
