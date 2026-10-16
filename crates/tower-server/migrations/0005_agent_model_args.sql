-- Spawn-time harness arguments (issue #4): the model id and extra flags
-- the agent was started with, for inspection (`tower ps --json`) and
-- respawn of named agents.

ALTER TABLE agents ADD COLUMN model TEXT;
ALTER TABLE agents ADD COLUMN args  TEXT NOT NULL DEFAULT '[]';