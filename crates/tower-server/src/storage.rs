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
/// (single-writer discipline, D§6). `seq` is AUTOINCREMENT — monotonic.
pub struct EventLog {
    write: tokio::sync::Mutex<sqlx::SqliteConnection>,
}

impl EventLog {
    pub async fn attach(pool: &SqlitePool) -> anyhow::Result<Self> {
        let conn = pool.acquire().await?.detach();
        Ok(Self {
            write: tokio::sync::Mutex::new(conn),
        })
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

        Ok(Event {
            seq: row.get::<i64, _>("seq"),
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
