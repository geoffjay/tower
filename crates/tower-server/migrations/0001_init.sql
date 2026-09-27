-- Initial schema (DESIGN.md §6, full job-queue columns included up front)

CREATE TABLE IF NOT EXISTS machines (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL UNIQUE,
  role        TEXT NOT NULL DEFAULT 'node',
  address     TEXT,
  status      TEXT NOT NULL DEFAULT 'offline',
  last_seen_at INTEGER,
  created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS agents (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL UNIQUE,
  kind          TEXT NOT NULL,
  machine_id    TEXT NOT NULL REFERENCES machines(id),
  pane_id       TEXT,
  workdir       TEXT,
  worktree      TEXT,
  state         TEXT NOT NULL DEFAULT 'unknown',
  desired_state TEXT NOT NULL DEFAULT 'running',
  permissions   TEXT NOT NULL DEFAULT 'default',
  adopted       INTEGER NOT NULL DEFAULT 0,
  config        TEXT NOT NULL DEFAULT '{}',
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tasks (
  id                TEXT PRIMARY KEY,
  agent_id          TEXT REFERENCES agents(id),
  owner_id          TEXT REFERENCES agents(id),
  origin            TEXT NOT NULL DEFAULT 'local',
  external_ref      TEXT,
  context_id        TEXT,
  title             TEXT NOT NULL,
  description       TEXT,
  state             TEXT NOT NULL DEFAULT 'queued',
  priority          INTEGER NOT NULL DEFAULT 0,
  tags              TEXT NOT NULL DEFAULT '[]',
  attempt_count     INTEGER NOT NULL DEFAULT 0,
  max_attempts      INTEGER NOT NULL DEFAULT 3,
  lease_expires_at  INTEGER,
  result            TEXT,
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tasks_agent ON tasks(agent_id);
CREATE INDEX IF NOT EXISTS idx_tasks_state ON tasks(state);
CREATE INDEX IF NOT EXISTS idx_tasks_pool ON tasks(state, priority DESC, created_at);

CREATE TABLE IF NOT EXISTS messages (
  id           TEXT PRIMARY KEY,
  task_id      TEXT REFERENCES tasks(id),
  from_kind    TEXT NOT NULL,
  from_id      TEXT NOT NULL,
  to_kind      TEXT NOT NULL,
  to_id        TEXT NOT NULL,
  kind         TEXT NOT NULL,
  parts        TEXT NOT NULL,
  status       TEXT NOT NULL DEFAULT 'delivered',
  deadline_at  INTEGER,
  responded_at INTEGER,
  created_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_to ON messages(to_kind, to_id, status);
CREATE INDEX IF NOT EXISTS idx_messages_task ON messages(task_id);

CREATE TABLE IF NOT EXISTS artifacts (
  id           TEXT PRIMARY KEY,
  task_id      TEXT REFERENCES tasks(id),
  name         TEXT NOT NULL,
  media_type    TEXT,
  content_path TEXT NOT NULL,
  size         INTEGER NOT NULL,
  created_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
  seq          INTEGER PRIMARY KEY AUTOINCREMENT,
  ts           INTEGER NOT NULL,
  type         TEXT NOT NULL,
  subject_type TEXT,
  subject_id   TEXT,
  payload      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_subject ON events(subject_type, subject_id);
CREATE INDEX IF NOT EXISTS idx_events_type ON events(type);

PRAGMA journal_mode=WAL;