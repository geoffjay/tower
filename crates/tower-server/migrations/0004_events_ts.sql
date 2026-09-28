-- Retention pruning and replay-from-time look events up by ts (D§17.6).
CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);
