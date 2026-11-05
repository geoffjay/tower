-- Workflows (D§5.6, D§6, D§9.7; decision: workflows).
-- A workflow is stored in versions; a run pins one; a step is one node
-- execution and owns at most one job (task_id is chosen before the job
-- exists, so it carries no FK). event_consumers holds the durable cursor of
-- in-process event-log consumers (the workflows engine).

CREATE TABLE workflows (
  id              TEXT PRIMARY KEY,
  name            TEXT NOT NULL UNIQUE,
  description     TEXT,
  enabled         INTEGER NOT NULL DEFAULT 1,
  current_version INTEGER NOT NULL,
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL
);

CREATE TABLE workflow_versions (
  workflow_id TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
  version     INTEGER NOT NULL,
  source      TEXT NOT NULL,
  definition  TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  PRIMARY KEY (workflow_id, version)
);

CREATE TABLE workflow_runs (
  id            TEXT PRIMARY KEY,
  workflow_id   TEXT NOT NULL,
  workflow_name TEXT NOT NULL,
  version       INTEGER NOT NULL,
  state         TEXT NOT NULL,
  input         TEXT NOT NULL,
  trigger       TEXT NOT NULL,
  result        TEXT,
  error         TEXT,
  created_at    INTEGER NOT NULL,
  ended_at      INTEGER
);
CREATE INDEX idx_runs_workflow ON workflow_runs(workflow_id, state);
CREATE INDEX idx_runs_state ON workflow_runs(state);

CREATE TABLE workflow_steps (
  id           TEXT PRIMARY KEY,
  run_id       TEXT NOT NULL REFERENCES workflow_runs(id),
  node_id      TEXT NOT NULL,
  visit        INTEGER NOT NULL,
  state        TEXT NOT NULL,
  activated_by TEXT NOT NULL DEFAULT '[]',
  task_id      TEXT UNIQUE,
  agent_id     TEXT,
  outcome      TEXT,
  result       TEXT,
  error        TEXT,
  created_at   INTEGER NOT NULL,
  started_at   INTEGER,
  ended_at     INTEGER,
  UNIQUE (run_id, node_id, visit)
);
CREATE INDEX idx_steps_state ON workflow_steps(state);

CREATE TABLE event_consumers (
  name   TEXT PRIMARY KEY,
  cursor INTEGER NOT NULL
);

-- a schedule owned by a workflow's schedule trigger starts runs, not jobs
ALTER TABLE schedules ADD COLUMN workflow_id    TEXT;
ALTER TABLE schedules ADD COLUMN workflow_input TEXT;

-- artifacts were never written; rebuild with the content in the database
DROP TABLE artifacts;
CREATE TABLE artifacts (
  id         TEXT PRIMARY KEY,
  task_id    TEXT NOT NULL REFERENCES tasks(id),
  name       TEXT NOT NULL,
  media_type TEXT,
  content    TEXT NOT NULL,
  size       INTEGER NOT NULL,
  created_by TEXT,
  created_at INTEGER NOT NULL
);
CREATE INDEX idx_artifacts_task ON artifacts(task_id);
