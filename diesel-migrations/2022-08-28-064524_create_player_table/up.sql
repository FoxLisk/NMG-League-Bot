CREATE TABLE IF NOT EXISTS players (
    id                   INTEGER PRIMARY KEY NOT NULL,
    name                 TEXT UNIQUE NOT NULL,
    discord_id           TEXT UNIQUE NOT NULL,
    racetime_username    TEXT UNIQUE NOT NULL,
    restreams_ok         INTEGER NOT NULL
);