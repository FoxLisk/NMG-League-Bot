-- Your SQL goes here
CREATE TABLE if not exists signups (
    id              INTEGER PRIMARY KEY NOT NULL,
    player_id       INTEGER NOT NULL,
    season_id       INTEGER NOT NULL,

    FOREIGN KEY(player_id) REFERENCES players(id),
    FOREIGN KEY(season_id) REFERENCES seasons(id)
);


CREATE UNIQUE INDEX player_season_signup ON signups(player_id, season_id);
