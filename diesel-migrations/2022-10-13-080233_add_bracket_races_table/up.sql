-- Your SQL goes here
CREATE TABLE if not exists bracket_races (
    id              INTEGER PRIMARY KEY NOT NULL,
    bracket_id      INTEGER NOT NULL, -- technically this is denormalized
    round_id        INTEGER NOT NULL,
    player_1_id     INTEGER NOT NULL,
    player_2_id     INTEGER NOT NULL,
    async_race_id   INTEGER NULL,
    state           TEXT NOT NULL,
    player_1_result TEXT NULL,
    player_2_result TEXT NULL,
    outcome         TEXT NULL,

    FOREIGN KEY(bracket_id) REFERENCES brackets(id),
    FOREIGN KEY(round_id) REFERENCES bracket_rounds(id),
    FOREIGN KEY(player_1_id) REFERENCES players(id),
    FOREIGN KEY(player_2_id) REFERENCES players(id),
    FOREIGN KEY(async_race_id) REFERENCES races(id)
);
