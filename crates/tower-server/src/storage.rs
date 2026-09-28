//! SQLite storage: single-writer pool + migrations + event log (D§6, T2.2/T2.3).

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tower_core::{Event, EventKind};

/// Run migrations from the embedded SQL files.
pub async fn open(db_file: &Path) -> anyhow::Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(db_file)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Event log writer: all writes go through one mutex-guarded connection
/// (single-writer discipline, D§6). `seq` is AUTOINCREMENT — monotonic,
/// never reused after pruning.
pub struct EventLog {
    write: tokio::sync::Mutex<sqlx::SqliteConnection>,
    /// Latest appended `seq`, for in-process followers (the web UI).
    head: tokio::sync::watch::Sender<i64>,
    /// Cursors of open `/v1/events` streams; pruning never passes them.
    cursors: parking_lot::Mutex<Vec<std::sync::Weak<std::sync::atomic::AtomicI64>>>,
}

/// A live stream's position, registered with [`EventLog::track_cursor`].
pub type CursorHandle = std::sync::Arc<std::sync::atomic::AtomicI64>;

impl EventLog {
    pub async fn attach(pool: &SqlitePool) -> anyhow::Result<Self> {
        let mut conn = pool.acquire().await?.detach();
        let head: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM events")
            .fetch_one(&mut conn)
            .await?;
        Ok(Self {
            write: tokio::sync::Mutex::new(conn),
            head: tokio::sync::watch::channel(head).0,
            cursors: parking_lot::Mutex::new(Vec::new()),
        })
    }

    /// Follow appends: the receiver sees the latest `seq`.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<i64> {
        self.head.subscribe()
    }

    /// Register a live stream's cursor; drop the handle to unregister.
    pub fn track_cursor(&self, cursor: i64) -> CursorHandle {
        let handle = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(cursor));
        let mut cursors = self.cursors.lock();
        cursors.retain(|w| w.strong_count() > 0);
        cursors.push(std::sync::Arc::downgrade(&handle));
        handle
    }

    /// Lowest cursor of an open stream, if any.
    fn min_live_cursor(&self) -> Option<i64> {
        let mut cursors = self.cursors.lock();
        cursors.retain(|w| w.strong_count() > 0);
        cursors
            .iter()
            .filter_map(|w| w.upgrade())
            .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
            .min()
    }

    /// Delete events older than `horizon` (epoch ms), except those an open
    /// stream has not read yet. Returns the number deleted (D§17.6).
    pub async fn prune(&self, horizon: i64) -> anyhow::Result<u64> {
        let floor = self.min_live_cursor().unwrap_or(i64::MAX);
        let mut conn = self.write.lock().await;
        let res = sqlx::query("DELETE FROM events WHERE ts < ?1 AND seq <= ?2")
            .bind(horizon)
            .bind(floor)
            .execute(&mut *conn)
            .await?;
        Ok(res.rows_affected())
    }

    /// Cursor that replays every event at or after `ts` (epoch ms).
    pub async fn cursor_at(&self, ts: i64) -> anyhow::Result<i64> {
        let mut conn = self.write.lock().await;
        let first: Option<i64> = sqlx::query_scalar("SELECT MIN(seq) FROM events WHERE ts >= ?1")
            .bind(ts)
            .fetch_one(&mut *conn)
            .await?;
        match first {
            Some(seq) => Ok(seq - 1),
            None => Ok(*self.head.borrow()),
        }
    }

    /// Append one event and return the stored row.
    pub async fn append(
        &self,
        kind: EventKind,
        subject_type: Option<&str>,
        subject_id: Option<&str>,
        payload: serde_json::Value,
    ) -> anyhow::Result<Event> {
        let now = tower_core::now_ms();
        let mut conn = self.write.lock().await;

        let row = sqlx::query(
            "INSERT INTO events (ts, type, subject_type, subject_id, payload)
             VALUES (?1, ?2, ?3, ?4, ?5)
             RETURNING seq",
        )
        .bind(now)
        .bind(kind.as_str())
        .bind(subject_type)
        .bind(subject_id)
        .bind(payload.to_string())
        .fetch_one(&mut *conn)
        .await?;
        let seq = row.get::<i64, _>("seq");
        // under the write lock, so followers see seqs in order
        self.head.send_replace(seq);
        drop(conn);

        Ok(Event {
            seq,
            ts: now,
            kind,
            subject_type: subject_type.map(str::to_string),
            subject_id: subject_id.map(str::to_string),
            payload,
        })
    }

    /// Replay events with `seq > cursor`, ordered, up to `limit`.
    pub async fn since(&self, cursor: i64, limit: i64) -> anyhow::Result<Vec<Event>> {
        let mut conn = self.write.lock().await;
        let rows = sqlx::query(
            "SELECT seq, ts, type, subject_type, subject_id, payload
             FROM events WHERE seq > ?1 ORDER BY seq ASC LIMIT ?2",
        )
        .bind(cursor)
        .bind(limit)
        .fetch_all(&mut *conn)
        .await?;

        rows.into_iter()
            .map(|r| {
                let kind_str: String = r.get("type");
                let kind: EventKind = serde_json::from_value(serde_json::Value::String(kind_str))?;
                Ok(Event {
                    seq: r.get("seq"),
                    ts: r.get("ts"),
                    kind,
                    subject_type: r.try_get("subject_type").ok(),
                    subject_id: r.try_get("subject_id").ok(),
                    payload: serde_json::from_str(&r.get::<String, _>("payload"))?,
                })
            })
            .collect()
    }

    pub async fn latest_seq(&self) -> anyhow::Result<i64> {
        let mut conn = self.write.lock().await;
        let row: Option<i64> = sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM events")
            .fetch_one(&mut *conn)
            .await?;
        Ok(row.unwrap_or(0))
    }
}

/// Enum columns are stored as their serde string form (kebab-case states).
pub fn enum_str<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Parse an enum column from its serde string form.
pub fn parse_enum<T: serde::de::DeserializeOwned>(s: String) -> T {
    serde_json::from_value(serde_json::Value::String(s)).expect("valid enum column")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_and_event_log() {
        let dir = std::env::temp_dir().join(format!("tower-test-{}", tower_core::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = open(&dir.join("test.db")).await.unwrap();
        let log = EventLog::attach(&pool).await.unwrap();

        assert_eq!(log.latest_seq().await.unwrap(), 0);

        let e1 = log
            .append(
                EventKind::ServerStarted,
                None,
                None,
                serde_json::json!({"version": "0.1.0"}),
            )
            .await
            .unwrap();
        let e2 = log
            .append(
                EventKind::AgentStateChange,
                Some("agent"),
                Some("a_1"),
                serde_json::json!({"from": "idle", "to": "working"}),
            )
            .await
            .unwrap();
        assert!(e2.seq > e1.seq, "seq must be monotonic");

        let replayed = log.since(0, 10).await.unwrap();
        assert_eq!(replayed.len(), 2);
        assert_eq!(replayed[0].kind, EventKind::ServerStarted);
        assert_eq!(replayed[1].kind, EventKind::AgentStateChange);

        let after_first = log.since(e1.seq, 10).await.unwrap();
        assert_eq!(after_first.len(), 1, "cursor excludes earlier events");

        std::fs::remove_dir_all(&dir).ok();
    }
}
