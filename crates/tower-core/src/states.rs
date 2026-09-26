//! State enums (D§5). `#[non_exhaustive]`: states evolve.

use serde::{Deserialize, Serialize};

/// Agent lifecycle, fed by herdr detection + tower's own launch tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AgentState {
    #[serde(rename = "unknown")]
    Unknown,
    #[serde(rename = "launching")]
    Launching,
    #[serde(rename = "idle")]
    Idle,
    #[serde(rename = "working")]
    Working,
    #[serde(rename = "blocked")]
    Blocked,
    #[serde(rename = "done")]
    Done,
    #[serde(rename = "dead")]
    Dead,
}

impl AgentState {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentState::Unknown => "unknown",
            AgentState::Launching => "launching",
            AgentState::Idle => "idle",
            AgentState::Working => "working",
            AgentState::Blocked => "blocked",
            AgentState::Done => "done",
            AgentState::Dead => "dead",
        }
    }

    /// Glyph for TUI/CLI rendering (docs/getting-started.md contract).
    pub fn glyph(&self) -> &'static str {
        match self {
            AgentState::Unknown => "?",
            AgentState::Launching => "◌",
            AgentState::Idle => "○",
            AgentState::Working => "●",
            AgentState::Blocked => "◉",
            AgentState::Done => "✓",
            AgentState::Dead => "✗",
        }
    }

    pub fn from_detection(s: &str) -> Option<Self> {
        // herdr PaneAgentState: idle/working/blocked/unknown
        // (herdr also has `done` in AgentStatus enum)
        match s {
            "idle" => Some(AgentState::Idle),
            "working" => Some(AgentState::Working),
            "blocked" => Some(AgentState::Blocked),
            "done" => Some(AgentState::Done),
            "unknown" => Some(AgentState::Unknown),
            _ => None,
        }
    }
}

/// What the operator wants the agent to be doing (row-level intent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesiredState {
    Running,
    Stopped,
}

/// Agent permission posture (D§8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permissions {
    Default,
    AcceptEdits,
    Yolo,
}

/// Task lifecycle (A2A-shaped, D§5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TaskState {
    #[serde(rename = "queued")]
    Queued,
    #[serde(rename = "working")]
    Working,
    #[serde(rename = "input-required")]
    InputRequired,
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "canceled")]
    Canceled,
    #[serde(rename = "rejected")]
    Rejected,
}

impl TaskState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskState::Completed | TaskState::Failed | TaskState::Canceled | TaskState::Rejected
        )
    }
}

/// Message kinds (unified table, D§5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MessageKind {
    Prompt,
    Question,
    Answer,
    Approval,
    ApprovalResponse,
    Notice,
    Delegation,
    Broadcast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MessageStatus {
    Pending,
    Delivered,
    Answered,
    Expired,
    Failed,
}

/// Endpoint kinds for message addressing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PartyKind {
    Human,
    Agent,
    Service,
    External,
    Room,
}

/// A2A Part content (D§5.3): exactly one content field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl Part {
    pub fn text(s: impl Into<String>) -> Self {
        Part {
            text: Some(s.into()),
            raw: None,
            url: None,
            data: None,
            media_type: None,
            filename: None,
            metadata: None,
        }
    }

    pub fn data(v: serde_json::Value) -> Self {
        Part {
            text: None,
            raw: None,
            url: None,
            data: Some(v),
            media_type: None,
            filename: None,
            metadata: None,
        }
    }
}

/// Alias kept for API symmetry with A2A naming.
pub type PartData = Part;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_state_detection_roundtrip() {
        for s in ["idle", "working", "blocked", "done", "unknown"] {
            assert!(AgentState::from_detection(s).is_some());
        }
        assert!(AgentState::from_detection("bogus").is_none());
    }

    #[test]
    fn task_state_terminal() {
        assert!(TaskState::Completed.is_terminal());
        assert!(!TaskState::Working.is_terminal());
    }

    #[test]
    fn part_serializes_minimal() {
        let p = Part::text("hello");
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["text"], "hello");
        assert!(v.get("raw").is_none() || v["raw"].is_null());
    }
}

/// Event types (D§5.4). Append-only log rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EventKind {
    #[serde(rename = "server.started")]
    ServerStarted,
    #[serde(rename = "agent.created")]
    AgentCreated,
    #[serde(rename = "agent.removed")]
    AgentRemoved,
    #[serde(rename = "agent.state")]
    AgentStateChange,
    #[serde(rename = "agent.output")]
    AgentOutput,
    #[serde(rename = "task.created")]
    TaskCreated,
    #[serde(rename = "task.status")]
    TaskStatus,
    #[serde(rename = "task.claimed")]
    TaskClaimed,
    #[serde(rename = "task.leased_out")]
    TaskLeasedOut,
    #[serde(rename = "task.completed")]
    TaskCompleted,
    #[serde(rename = "task.failed")]
    TaskFailed,
    #[serde(rename = "message.created")]
    MessageCreated,
    #[serde(rename = "message.status")]
    MessageStatusChange,
    #[serde(rename = "approval.expired")]
    ApprovalExpired,
    #[serde(rename = "machine.state")]
    MachineState,
    #[serde(rename = "node.registered")]
    NodeRegistered,
    #[serde(rename = "node.disconnected")]
    NodeDisconnected,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
            .leak()
    }
}
