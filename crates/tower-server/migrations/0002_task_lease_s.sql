-- Per-task lease window (D§5.2.1, D§6): each assign/start/heartbeat extends
-- ownership by lease_s seconds.
ALTER TABLE tasks ADD COLUMN lease_s INTEGER NOT NULL DEFAULT 60;
