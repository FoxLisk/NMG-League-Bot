-- Your SQL goes here
CREATE TABLE if not exists seasons (
    id           INTEGER PRIMARY KEY NOT NULL,
    started      BIGINT NOT NULL,
    finished     BIGINT NULL,
    format       TEXT NOT NULL
);