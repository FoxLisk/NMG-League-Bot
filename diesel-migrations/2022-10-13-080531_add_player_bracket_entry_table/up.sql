-- represents a player being in a particular bracket
CREATE TABLE if not exists player_bracket_entry (
    id              INTEGER PRIMARY KEY NOT NULL,
    bracket_id      INTEGER NOT NULL,
    player_id       INTEGER NOT NULL,
    -- at some point we probably want like (dropped)
    FOREIGN KEY(bracket_id) REFERENCES brackets(id),
    FOREIGN KEY(player_id) REFERENCES players(id)
);
