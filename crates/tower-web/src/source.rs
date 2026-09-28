//! What the UI reads (D§12.3): the server implements this over its
//! database and event log; tests implement it over fixtures. Plain
//! `tower-core` types only — nothing Topcoat crosses this line.

use tower_core::{Agent, Event, Machine, Message, Task};

/// Current state, re-read on every render.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub machines: Vec<Machine>,
    pub agents: Vec<Agent>,
    /// Jobs not yet terminal.
    pub open_tasks: Vec<Task>,
    /// Pending questions/approvals sent by agents (the operator's inbox).
    pub pending_from_agents: Vec<Message>,
}

#[async_trait::async_trait]
pub trait UiSource: Send + Sync + 'static {
    async fn snapshot(&self) -> anyhow::Result<Snapshot>;

    /// Events with `seq > cursor`, ascending, at most `limit`.
    async fn events_since(&self, cursor: i64, limit: i64) -> anyhow::Result<Vec<Event>>;

    /// A cursor that replays every event at or after `ts` (epoch ms).
    async fn cursor_at(&self, ts: i64) -> anyhow::Result<i64>;

    /// The latest appended `seq`; changes wake the UI's follower.
    fn head(&self) -> tokio::sync::watch::Receiver<i64>;

    /// Epoch ms. Injected so tests control windows and countdowns.
    fn now(&self) -> i64 {
        tower_core::now_ms()
    }
}
