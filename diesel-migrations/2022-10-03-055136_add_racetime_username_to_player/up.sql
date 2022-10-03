-- Your SQL goes here
CREATE TABLE __new_players (
    id                 INTEGER PRIMARY KEY NOT NULL,
    name               TEXT NOT NULL,
    discord_id         TEXT UNIQUE NOT NULL,
    racetime_username  TEXT UNIQUE NOT NULL,
    restreams_ok       INTEGER NOT NULL
);

INSERT INTO __new_players (id, name, discord_id, racetime_username, restreams_ok)
SELECT                     id, name, discord_id, 'unset#0000',      restreams_ok
FROM players;

DROP TABLE players;

ALTER TABLE __new_players RENAME TO players;
