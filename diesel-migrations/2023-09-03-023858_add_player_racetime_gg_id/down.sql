ROLLBACK;
PRAGMA foreign_keys=OFF;
BEGIN;
CREATE TABLE __new_players
(
    id                   INTEGER PRIMARY KEY NOT NULL,
    name                 TEXT UNIQUE NOT NULL COLLATE NOCASE,
    discord_id           TEXT UNIQUE NOT NULL,
    racetime_username    TEXT UNIQUE NULL COLLATE NOCASE,
    twitch_user_login    TEXT UNIQUE NULL COLLATE NOCASE
);

INSERT INTO __new_players(id, name, discord_id, racetime_username, twitch_user_login )
SELECT                    id, name, discord_id, racetime_username, twitch_user_login
FROM players;


DROP TABLE players;
ALTER TABLE __new_players RENAME TO players;

PRAGMA foreign_key_check;
PRAGMA foreign_keys=ON;