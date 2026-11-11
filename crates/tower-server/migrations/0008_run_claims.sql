-- Run claims (D§5.6, D§6; decision: agent-availability). A workflow run
-- claims every agent its definition names before its first step, all or
-- nothing; the agent is the key, so one run holds an agent at a time.
-- A claimed agent gets only its run's jobs. Ending the run deletes its rows;
-- removing the agent deletes its row first (the FK must go).

CREATE TABLE run_claims (
  agent_id   TEXT PRIMARY KEY REFERENCES agents(id),
  run_id     TEXT NOT NULL REFERENCES workflow_runs(id),
  claimed_at INTEGER NOT NULL
);
CREATE INDEX idx_claims_run ON run_claims(run_id);
