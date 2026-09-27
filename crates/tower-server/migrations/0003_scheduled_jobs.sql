-- Scheduled and recurring jobs (D§5.2.2, D§6; decision: scheduled-jobs).

-- reserved delivery + one-off holds + schedule provenance
ALTER TABLE tasks ADD COLUMN target_agent_id TEXT REFERENCES agents(id);
ALTER TABLE tasks ADD COLUMN not_before INTEGER;
-- no FK: a job keeps its schedule id as history after the schedule is removed
ALTER TABLE tasks ADD COLUMN schedule_id TEXT;
ALTER TABLE tasks ADD COLUMN occurrence_at INTEGER;
CREATE INDEX IF NOT EXISTS idx_tasks_target ON tasks(target_agent_id, state);
-- exactly-once firing (NULLs stay distinct for non-schedule jobs)
CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_occurrence ON tasks(schedule_id, occurrence_at);

CREATE TABLE IF NOT EXISTS schedules (
  id               TEXT PRIMARY KEY,
  title            TEXT NOT NULL,
  description      TEXT,
  tags             TEXT NOT NULL DEFAULT '[]',
  priority         INTEGER NOT NULL DEFAULT 0,
  lease_s          INTEGER NOT NULL DEFAULT 60,
  max_attempts     INTEGER NOT NULL DEFAULT 3,
  target_agent_id  TEXT REFERENCES agents(id),
  cron             TEXT NOT NULL,
  timezone         TEXT NOT NULL,
  enabled          INTEGER NOT NULL DEFAULT 1,
  next_run_at      INTEGER,
  last_run_at      INTEGER,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_schedules_due ON schedules(enabled, next_run_at);
