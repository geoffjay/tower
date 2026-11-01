-- Job lifecycle prompts (D§5.2.3, D§6; decision: job-lifecycle-hooks).
-- pre_run is typed into the harness before the job prompt on every
-- delivery, post_run after the owner closes the job; hook marks the one
-- in flight ('pre' pauses lease expiry, 'post' keeps the owner busy).

ALTER TABLE tasks ADD COLUMN pre_run  TEXT;
ALTER TABLE tasks ADD COLUMN post_run TEXT;
ALTER TABLE tasks ADD COLUMN hook     TEXT;

-- copied into every job the schedule fires
ALTER TABLE schedules ADD COLUMN pre_run  TEXT;
ALTER TABLE schedules ADD COLUMN post_run TEXT;
