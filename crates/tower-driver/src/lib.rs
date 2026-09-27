//! Harness abstraction (D§8.1, plan T4.1).
//!
//! One implementation in phase 1: HerdrDriver (CLI transport). TmuxDriver
//! (fallback) and the remote node proxy arrive later; the trait is the seam.

pub mod fake;
pub mod herdr;

use async_trait::async_trait;
use futures::stream::BoxStream;

/// How to spawn an agent.
#[derive(Debug, Clone)]
pub struct AgentSpec {
    pub name: String,
    pub kind: String,
    pub workdir: Option<String>,
    /// Extra args appended after the harness executable.
    pub args: Vec<String>,
    /// Environment for the agent's pane (e.g. `TOWER_AGENT`, D§7).
    pub env: Vec<(String, String)>,
}

/// What a driver read from an agent's terminal.
#[derive(Debug, Clone)]
pub struct ReadResult {
    /// Plain text by default; ANSI when requested.
    pub text: String,
}

/// Where to read terminal state from (S1.A grammar: herdr read --source).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadSource {
    Visible,
    Recent,
    RecentUnwrapped,
    Detection,
}

impl ReadSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReadSource::Visible => "visible",
            ReadSource::Recent => "recent",
            ReadSource::RecentUnwrapped => "recent-unwrapped",
            ReadSource::Detection => "detection",
        }
    }
}

/// Harness-detected lifecycle states (herdr PaneAgentState superset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessState {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl HarnessState {
    pub fn from_detection(s: &str) -> Option<Self> {
        match s {
            "idle" => Some(HarnessState::Idle),
            "working" => Some(HarnessState::Working),
            "blocked" => Some(HarnessState::Blocked),
            "done" => Some(HarnessState::Done),
            "unknown" => Some(HarnessState::Unknown),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            HarnessState::Idle => "idle",
            HarnessState::Working => "working",
            HarnessState::Blocked => "blocked",
            HarnessState::Done => "done",
            HarnessState::Unknown => "unknown",
        }
    }
}

/// An agent as the harness sees it (snapshot entries).
#[derive(Debug, Clone)]
pub struct HarnessAgent {
    pub name: String,
    pub kind: String,
    pub pane_id: String,
    pub state: HarnessState,
    pub cwd: Option<String>,
}

/// Events surfaced by a driver (plan T4.3 pump consumes these).
#[derive(Debug, Clone)]
pub enum HarnessEvent {
    /// Agent appeared (started or adopted).
    AgentUp(HarnessAgent),
    /// Agent state transition with detection detail.
    StateChange {
        name: String,
        from: HarnessState,
        to: HarnessState,
        detail: Option<String>,
    },
    /// Terminal output chunk.
    Output { name: String, text: String },
    /// Agent gone (pane closed / process died).
    AgentDown { name: String, pane_id: String },
}

#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// herdr returned an error envelope (code preserved).
    #[error("herdr error {code}: {message}")]
    Herdr { code: String, message: String },
    /// Non-zero exit / unparseable output.
    #[error("driver transport failure: {0}")]
    Transport(String),
    /// Target (agent/pane) not found.
    #[error("not found: {0}")]
    NotFound(String),
    /// Prompt was sent but settled-state wait stalled (S1.A: NOT fatal —
    /// re-read output and surface text; agent may be at an error screen).
    #[error("prompt accepted but no state change observed: {0}")]
    PromptStalled(String),
    /// Wait/prompt timeout.
    #[error("timeout: {0}")]
    Timeout(String),
    /// Agent is blocked; input rejected.
    #[error("agent blocked, input rejected: {0}")]
    AgentBlocked(String),
}

/// The driver contract (D§8.1). Object-safe; events via BoxStream.
#[async_trait]
pub trait Harness: Send + Sync {
    /// List agents the harness knows about (inventory reconcile source).
    async fn snapshot(&self) -> Result<Vec<HarnessAgent>, DriverError>;

    /// Start an agent in a fresh pane; returns the agent's pane id.
    async fn start(&self, spec: &AgentSpec) -> Result<String, DriverError>;

    /// Send a prompt. `wait` = wait for a settled state (idle/done/blocked).
    async fn prompt(&self, name: &str, text: &str, wait: bool) -> Result<(), DriverError>;

    /// Interrupt (ctrl+c semantics).
    async fn interrupt(&self, name: &str) -> Result<(), DriverError>;

    /// Send raw keys to the agent's pane, in order (approval answers `1`/`2`,
    /// D§8.2; power-user escape hatch).
    async fn send_keys(&self, name: &str, keys: &[String]) -> Result<(), DriverError>;

    /// Read terminal output.
    async fn read(
        &self,
        name: &str,
        source: ReadSource,
        ansi: bool,
    ) -> Result<ReadResult, DriverError>;

    /// Wait until the agent reaches one of the given states.
    async fn wait_until(
        &self,
        name: &str,
        states: &[HarnessState],
        timeout_ms: u64,
    ) -> Result<(), DriverError>;

    /// Stop the agent's pane (session ends; herdr keeps workspace layout).
    async fn stop(&self, name: &str) -> Result<(), DriverError>;

    /// Event pump (state changes + output); driver-owned polling.
    fn events(&self) -> BoxStream<'static, HarnessEvent>;
}
