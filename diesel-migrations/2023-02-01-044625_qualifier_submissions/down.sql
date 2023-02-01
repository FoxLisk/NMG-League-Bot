-- This file should undo anything in `up.sql`
DROP TABLE qualifier_submissions;
ALTER TABLE seasons DROP COLUMN state;