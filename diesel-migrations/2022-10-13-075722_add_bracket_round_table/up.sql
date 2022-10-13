-- represents a round in a bracket
CREATE TABLE if not exists bracket_rounds (
    id              INTEGER PRIMARY KEY NOT NULL,
    round_num       INTEGER NOT NULL,
    bracket_id      INTEGER NOT NULL,
    -- at some point we probably want like (dropped)
    FOREIGN KEY(bracket_id) REFERENCES brackets(id)
);

CREATE UNIQUE INDEX round_num_uniq ON bracket_rounds(round_num, bracket_id);