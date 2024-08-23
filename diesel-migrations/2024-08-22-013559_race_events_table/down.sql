-- This file should undo anything in `up.sql`
ALTER TABLE bracket_race_infos ADD COLUMN scheduled_event_id TEXT NULL;

UPDATE bracket_race_infos 
SET scheduled_event_id = re.scheduled_event_id
FROM (SELECT scheduled_event_id, bracket_race_info_id FROM race_events) as re
WHERE bracket_race_infos.id = re.bracket_race_info_id;

DROP TABLE race_events;