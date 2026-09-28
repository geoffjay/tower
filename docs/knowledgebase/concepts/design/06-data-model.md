---
type: Concept
title: Design §6 — Data model (DDL)
description: SQLite schema for machines, agents, tasks, messages, artifacts, and events; single-writer discipline.
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
  at: "2026-09-26T22:59:25Z"
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

CREATE TABLE artifacts (
  id           TEXT PRIMARY KEY,
  task_id      TEXT REFERENCES tasks(id),
  name         TEXT NOT NULL,
  media_type   TEXT,
  content_path TEXT NOT NULL,  -- file under artifacts/
  size         INTEGER NOT NULL,
  created_at   INTEGER NOT NULL
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
