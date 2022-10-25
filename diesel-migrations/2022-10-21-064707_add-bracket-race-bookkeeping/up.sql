-- represents administrative bookkeeping around a race (when is it scheduled,
-- what is its race room, etc)
-- i hate this name but i cant think of a better one
CREATE TABLE if not exists bracket_race_infos (
    id                           INTEGER PRIMARY KEY NOT NULL,
    bracket_race_id              INTEGER UNIQUE NOT NULL,
    scheduled_for                BIGINT NULL,
    scheduled_event_id           TEXT NULL,
    commportunities_message_id   TEXT NULL,
    restream_request_message_id  TEXT NULL,
    racetime_gg_url              TEXT NULL,
    -- at some point we probably want like (dropped)
    FOREIGN KEY(bracket_race_id) REFERENCES bracket_races(id)
);
