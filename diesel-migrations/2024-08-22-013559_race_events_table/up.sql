CREATE TABLE race_events (
   id                   INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
   guild_id             TEXT NOT NULL,
   bracket_race_info_id INTEGER NOT NULL,
   scheduled_event_id   TEXT NOT NULL UNIQUE,

   FOREIGN KEY(bracket_race_info_id) REFERENCES bracket_race_infos(id) 
);

CREATE UNIQUE INDEX guild_bri ON race_events(guild_id, bracket_race_info_id);

INSERT INTO race_events (guild_id,             bracket_race_info_id, scheduled_event_id)
                         -- NOTE: this is the prod server id!
SELECT                   "982502396590698498", id,                   scheduled_event_id
FROM bracket_race_infos
WHERE scheduled_event_id IS NOT NULL;

ALTER TABLE bracket_race_infos DROP COLUMN scheduled_event_id;