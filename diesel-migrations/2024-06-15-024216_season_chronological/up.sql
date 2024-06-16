ROLLBACK;
PRAGMA foreign_keys=OFF;
BEGIN;
CREATE TABLE __new_seasons
(
   id                   INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
   started              BIGINT NOT NULL,
   finished             BIGINT NULL,
   format               TEXT NOT NULL,
   ordinal              INTEGER NOT NULL UNIQUE,
   state                TEXT NOT NULL DEFAULT '"Created"',
   rtgg_category_name   TEXT NOT NULL DEFAULT "alttp",
   rtgg_goal_name       TEXT NOT NULL DEFAULT "Any% NMG"
);

INSERT INTO __new_seasons(id, started, finished, format, ordinal, state, rtgg_category_name, rtgg_goal_name)
SELECT                    id, started, finished, format, id,      state, rtgg_category_name, rtgg_goal_name
FROM seasons;


DROP TABLE seasons;
ALTER TABLE __new_seasons RENAME TO seasons;

PRAGMA foreign_key_check;
PRAGMA foreign_keys=ON;