-- Your SQL goes here
CREATE TABLE guild_race_criteria (
    id              INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    guild_id        TEXT NOT NULL,
    player_id       INTEGER NULL,
    restream_status BOOLEAN NULL,

    FOREIGN KEY(player_id) REFERENCES players(id) 
);