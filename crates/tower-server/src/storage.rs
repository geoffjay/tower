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
        rows.iter().map(row_event).collect()
    }

    /// One page of the log, newest first (D§7 `/v1/events/history`): at
    /// most `limit` events matching `f` with `seq < before` (`None`: from
    /// the head). `head` is read under the same lock as the page, so a
    /// stream from `cursor = head` continues it without a gap or overlap.
    pub async fn history(
        &self,
        before: Option<i64>,
        limit: i64,
        f: &EventFilter,
    ) -> anyhow::Result<EventPage> {
        let mut conn = self.write.lock().await;
        let head = *self.head.borrow();
        let before = before.map_or(head + 1, |b| b.min(head + 1));
        // One extra row tells whether an older match exists.
        let rows = sqlx::query(
            "SELECT seq, ts, type, subject_type, subject_id, payload
             FROM events
             WHERE seq < ?1
               AND (?2 IS NULL OR instr(type, ?2) > 0)
               AND (?3 IS NULL OR instr(type, ?3) = 0)
               AND (?4 IS NULL OR lower(subject_type || ':' || subject_id) = lower(?4))
             ORDER BY seq DESC LIMIT ?5",
        )
        .bind(before)
        .bind(f.filter())
        .bind(f.exclude())
        .bind(f.subject())
        .bind(limit + 1)
        .fetch_all(&mut *conn)
        .await?;
        drop(conn);
        let more = rows.len() as i64 > limit;
        let events = rows
            .iter()
            .take(limit as usize)
            .map(row_event)
            .collect::<anyhow::Result<_>>()?;
        Ok(EventPage { events, head, more })
    }

    pub async fn latest_seq(&self) -> anyhow::Result<i64> {
        let mut conn = self.write.lock().await;
        let row: Option<i64> = sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) FROM events")
            .fetch_one(&mut *conn)
            .await?;
        Ok(row.unwrap_or(0))
    }
}

fn row_event(r: &sqlx::sqlite::SqliteRow) -> anyhow::Result<Event> {
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
}

/// A page of [`EventLog::history`]: newest first.
#[derive(Debug, serde::Serialize)]
pub struct EventPage {
    pub events: Vec<Event>,
    /// The log's latest seq when the page was read: the live cursor.
    pub head: i64,
    /// Older matching events exist (page on with `before`).
    pub more: bool,
}

/// Which events a reader wants (D§7), the same for the stream and the
/// history: `filter` keeps event types containing its text, `exclude`
/// drops them (a `type:` prefix is optional), `subject` is an exact
/// `type:id`, ASCII case-insensitive. Empty values are no filter.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub filter: Option<String>,
    pub exclude: Option<String>,
    pub subject: Option<String>,
}

impl EventFilter {
    fn type_needle(v: &Option<String>) -> Option<&str> {
        let v = v.as_deref()?;
        Some(v.strip_prefix("type:").unwrap_or(v)).filter(|s| !s.is_empty())
    }

    pub fn filter(&self) -> Option<&str> {
        Self::type_needle(&self.filter)
    }

    pub fn exclude(&self) -> Option<&str> {
        Self::type_needle(&self.exclude)
    }

    pub fn subject(&self) -> Option<&str> {
        self.subject.as_deref().filter(|s| !s.is_empty())
    }

    pub fn matches(&self, e: &Event) -> bool {
        let kind = e.kind.as_str();
        if self.filter().is_some_and(|f| !kind.contains(f)) {
            return false;
        }
        if self.exclude().is_some_and(|x| kind.contains(x)) {
            return false;
        }
        if let Some(want) = self.subject() {
            let got = match (e.subject_type.as_deref(), e.subject_id.as_deref()) {
                (Some(t), Some(id)) => format!("{t}:{id}"),
                _ => return false,
            };
            if !got.eq_ignore_ascii_case(want) {
                return false;
            }
        }
        true
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

    /// sqlx refuses to start on a database whose applied migration no
    /// longer matches its file ("migration N was previously applied but
    /// has been modified") — and it checksums the whole file, comments
    /// included. Shipped migrations are therefore frozen: change the
    /// schema with a new migration, then add its checksum here.
    #[test]
    fn applied_migrations_are_never_edited() {
        const PINNED: &[(i64, &str)] = &[
            (1, "9d4cace00f87ef7b2eca1e6c7ebc76e21df07424fc608c320fa96984257c9640656ea1b37ca25f7faa405f8a3d00fcc5"),
            (2, "1bd6e1cc770cb0b9b49dfb50935dda5ab687aacf1e4b814cc6ffa0d442910490dfac096bd687be1d51a7d72c5dc48000"),
            (3, "f5f7e03ae540168951ef6715d73f776d1e417503fcb381c0b622501900cf228fa6c74780eb9ce696ed715b9aa0f37e99"),
            (4, "48f64541fb76afce36672da5867964280010e7b5ba57076a9c17c28cdfa1cb4f5e9d5c9a20ed408870f8d7f2e39048cc"),
            (5, "4dcfa85265ec1e66c0dbcf3582c5634ed4d8fded8f252febcb9b097bbc5e1b88af1df7df5dc3fd6f55f9d26c792e1f4b"),
            (6, "7d92d2302e1a553a40328b411434dcdf92e78efdf86ec3de1544b2326a349ee8b1da992631e06379f3500b773b961255"),
        ];
        let found: Vec<(i64, String)> = sqlx::migrate!("./migrations")
            .iter()
            .map(|m| {
                let hex = m.checksum.iter().map(|b| format!("{b:02x}")).collect();
                (m.version, hex)
            })
            .collect();
        for (version, sum) in PINNED {
            let got = found.iter().find(|(v, _)| v == version);
            assert_eq!(
                got.map(|(_, s)| s.as_str()),
                Some(*sum),
                "migration {version} changed after it shipped; add a new migration instead"
            );
        }
        assert_eq!(
            found.len(),
            PINNED.len(),
            "new migration: pin its checksum in PINNED"
        );
    }

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
