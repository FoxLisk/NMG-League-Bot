
CREATE TABLE if not exists commentator_signups (
    id                           INTEGER PRIMARY KEY NOT NULL,
    bracket_race_info_id         INTEGER NOT NULL,
    discord_id                   TEXT NOT NULL,
    -- at some point we probably want like (dropped)
    FOREIGN KEY(bracket_race_info_id) REFERENCES bracket_race_infos(id)
);

CREATE UNIQUE INDEX signup_race ON commentator_signups(bracket_race_info_id, discord_id);