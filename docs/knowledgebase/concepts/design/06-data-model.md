---
type: Concept
title: Design §6 — Data model (DDL)
description: SQLite schema for machines, agents, tasks, schedules, messages, artifacts, workflows, and events; single-writer discipline.
tags:
  - design
  - design-s6
  - data-model
  - sqlite
  - schema
status: draft
sources:
  - resource: git:340c189:DESIGN.md
    title: tower design document §6 (original, removed from repo root after ingest)
generated:
  by: omp/claude-opus-5-5
  at: "2026-10-31T12:00:00Z"
---

# 6. Data model (DDL)

```sql
CREATE TABLE machines (
  id          TEXT PRIMARY KEY,          -- ULID
  name        TEXT NOT NULL UNIQUE,
  role        TEXT NOT NULL DEFAULT 'node',
  address     TEXT,                      -- host:port or ssh alias
  status      TEXT NOT NULL DEFAULT 'offline',
  last_seen_at INTEGER,
  created_at  INTEGER NOT NULL
);

CREATE TABLE agents (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL UNIQUE,
  kind          TEXT NOT NULL,           -- claude | pi | (herdr kind)
  machine_id    TEXT NOT NULL REFERENCES machines(id),
  pane_id       TEXT,                    -- herdr pane identifier
  workdir       TEXT,
  worktree      TEXT,                    -- isolated worktree, or NULL
  state         TEXT NOT NULL DEFAULT 'unknown',
  desired_state TEXT NOT NULL DEFAULT 'running',   -- running | stopped
  permissions   TEXT NOT NULL DEFAULT 'default',   -- default | accept-edits | yolo
  config        TEXT NOT NULL DEFAULT '{}',
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE TABLE tasks (
  id                TEXT PRIMARY KEY,
  agent_id          TEXT REFERENCES agents(id),   -- creator/responsible agent (optional)
  owner_id          TEXT REFERENCES agents(id),  -- current exclusive owner (NULL = in queue, awaiting assignment)
  origin            TEXT NOT NULL DEFAULT 'local',  -- local | a2a | node:<machine>
  external_ref      TEXT,                          -- A2A task id when origin=a2a
  context_id        TEXT,                          -- A2A context grouping
  title             TEXT NOT NULL,
  description       TEXT,
  state             TEXT NOT NULL DEFAULT 'queued',
  priority          INTEGER NOT NULL DEFAULT 0,
  tags              TEXT NOT NULL DEFAULT '[]',   -- JSON array
  attempt_count     INTEGER NOT NULL DEFAULT 0,
  max_attempts      INTEGER NOT NULL DEFAULT 3,
  lease_expires_at  INTEGER,                      -- ownership deadline; NULL when unowned
  lease_s           INTEGER NOT NULL DEFAULT 60,   -- renewal window per assign/heartbeat (migration 0002)
  target_agent_id   TEXT REFERENCES agents(id),    -- reserved for this agent; dispatcher delivers when available (0003)
  not_before        INTEGER,                       -- dispatcher holds the job until this time (0003)
  schedule_id       TEXT,                          -- schedule that materialized it; no FK: history survives schedule removal (0003)
  occurrence_at     INTEGER,                       -- the schedule time this job is for (0003)
  pre_run           TEXT,                          -- lifecycle prompt typed before the job prompt, every delivery (0006)
  post_run          TEXT,                          -- lifecycle prompt typed after the owner closes the job (0006)
  hook              TEXT,                          -- lifecycle prompt in flight: 'pre' | 'post' | NULL (0006, D§5.2.3)
  result            TEXT,                          -- JSON summary
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL
);
CREATE INDEX idx_tasks_agent ON tasks(agent_id);
CREATE INDEX idx_tasks_state ON tasks(state);
CREATE INDEX idx_tasks_pool ON tasks(state, priority DESC, created_at);
CREATE INDEX idx_tasks_target ON tasks(target_agent_id, state);
CREATE UNIQUE INDEX idx_tasks_occurrence ON tasks(schedule_id, occurrence_at);  -- exactly-once firing

CREATE TABLE schedules (                          -- migration 0003
  id               TEXT PRIMARY KEY,
  title            TEXT NOT NULL,
  description      TEXT,
  tags             TEXT NOT NULL DEFAULT '[]',
  priority         INTEGER NOT NULL DEFAULT 0,
  lease_s          INTEGER NOT NULL DEFAULT 60,
  max_attempts     INTEGER NOT NULL DEFAULT 3,
  target_agent_id  TEXT REFERENCES agents(id),     -- NULL = jobs go to the general queue
  cron             TEXT NOT NULL,                   -- 5/6-field cron (croner)
  timezone         TEXT NOT NULL,                   -- IANA name, fixed at creation
  enabled          INTEGER NOT NULL DEFAULT 1,
  next_run_at      INTEGER,                         -- CAS-advanced on each firing
  last_run_at      INTEGER,
  pre_run          TEXT,                            -- copied into every job it fires (0006)
  post_run         TEXT,                            -- (0006)
  workflow_id      TEXT,                            -- managed: fires a workflow run instead of a job (0007, D§5.6)
  workflow_input   TEXT,                            -- JSON input for the run it starts (0007)
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
);

CREATE TABLE messages (
  id          TEXT PRIMARY KEY,
  task_id     TEXT REFERENCES tasks(id),
  from_kind   TEXT NOT NULL,   -- human | agent | service | external
  from_id     TEXT NOT NULL,
  to_kind     TEXT NOT NULL,   -- human | agent | room | external
  to_id       TEXT NOT NULL,
  kind        TEXT NOT NULL,
  parts       TEXT NOT NULL,    -- JSON array of parts
  status      TEXT NOT NULL DEFAULT 'delivered',
  deadline_at INTEGER,
  responded_at INTEGER,
  created_at  INTEGER NOT NULL
);
CREATE INDEX idx_messages_to ON messages(to_kind, to_id, status);
CREATE INDEX idx_messages_task ON messages(task_id);

CREATE TABLE artifacts (                          -- rebuilt in 0007 (never written before)
  id           TEXT PRIMARY KEY,
  task_id      TEXT NOT NULL REFERENCES tasks(id),
  name         TEXT NOT NULL,
  media_type   TEXT,
  content      TEXT NOT NULL,                     -- UTF-8 text, ≤ 1 MiB
  size         INTEGER NOT NULL,                  -- bytes
  created_by   TEXT,                              -- the owning agent's id
  created_at   INTEGER NOT NULL
);
CREATE INDEX idx_artifacts_task ON artifacts(task_id);

CREATE TABLE workflows (                          -- 0007, D§5.6
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
  source      TEXT NOT NULL,                      -- TOML as applied
  definition  TEXT NOT NULL,                      -- validated graph, JSON
  created_at  INTEGER NOT NULL,
  PRIMARY KEY (workflow_id, version)
);

CREATE TABLE workflow_runs (
  id            TEXT PRIMARY KEY,                 -- = tasks.context_id of its jobs
  workflow_id   TEXT NOT NULL,                    -- no FK: history survives removal
  workflow_name TEXT NOT NULL,
  version       INTEGER NOT NULL,
  state         TEXT NOT NULL,                    -- running | completed | failed | canceled
  input         TEXT NOT NULL,                    -- JSON object
  trigger       TEXT NOT NULL,                    -- JSON {kind, schedule_id?, event_seq?, trigger?}
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
  state        TEXT NOT NULL,                     -- pending | ready | running | completed | failed | canceled
  activated_by TEXT NOT NULL DEFAULT '[]',        -- JSON array of step ids whose edges fired
  task_id      TEXT UNIQUE,                       -- chosen before the job exists (no FK)
  agent_id     TEXT,
  outcome      TEXT,
  result       TEXT,                              -- {summary, data}, frozen at close
  error        TEXT,
  created_at   INTEGER NOT NULL,
  started_at   INTEGER,
  ended_at     INTEGER,
  UNIQUE (run_id, node_id, visit)
);
CREATE INDEX idx_steps_state ON workflow_steps(state);

CREATE TABLE event_consumers (                    -- durable in-process event cursors (D§9.6)
  name   TEXT PRIMARY KEY,
  cursor INTEGER NOT NULL
);

CREATE TABLE events (
  seq          INTEGER PRIMARY KEY AUTOINCREMENT,
  ts           INTEGER NOT NULL,
  type         TEXT NOT NULL,
  subject_type TEXT,
  subject_id   TEXT,
  payload      TEXT NOT NULL
);
CREATE INDEX idx_events_subject ON events(subject_type, subject_id);
CREATE INDEX idx_events_type ON events(type);

PRAGMA journal_mode=WAL;
```

Single-writer discipline: one write connection behind a tokio mutex; readers on
the pool. Write volume is low (state changes, message posts), so this is not a
bottleneck.

Migrations (`crates/tower-server/migrations/`, embedded with `sqlx::migrate!`)
are frozen once shipped. sqlx checksums the whole file, comments included,
and refuses to start a database whose applied migration no longer matches.
Change the schema with a new numbered migration; a test pins every shipped
checksum (`storage::tests::applied_migrations_are_never_edited`).
