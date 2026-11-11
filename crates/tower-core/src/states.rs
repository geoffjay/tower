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

/// Task lifecycle (A2A-shaped + explicit `assigned`, D§5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TaskState {
    #[serde(rename = "queued")]
    Queued,
    #[serde(rename = "assigned")]
    Assigned,
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

/// Which lifecycle prompt is in flight on a job (D§5.2.3): `pre` before
/// its job prompt, `post` after its owner closed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HookPhase {
    Pre,
    Post,
}

impl HookPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            HookPhase::Pre => "pre",
            HookPhase::Post => "post",
        }
    }
}

/// Workflow run lifecycle (D§5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    /// Created, but an agent it claims is held by another run (D§5.6).
    Waiting,
    Running,
    Completed,
    Failed,
    Canceled,
}

impl RunState {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunState::Waiting => "waiting",
            RunState::Running => "running",
            RunState::Completed => "completed",
            RunState::Failed => "failed",
            RunState::Canceled => "canceled",
        }
    }
}

/// Workflow step lifecycle (D§5.6): `pending` is a `join = "all"` node
/// collecting fired edges; `ready` has its job id chosen but not running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepState {
    Pending,
    Ready,
    Running,
    Completed,
    Failed,
    Canceled,
}

impl StepState {
    pub fn as_str(&self) -> &'static str {
        match self {
            StepState::Pending => "pending",
            StepState::Ready => "ready",
            StepState::Running => "running",
            StepState::Completed => "completed",
            StepState::Failed => "failed",
            StepState::Canceled => "canceled",
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(
            self,
            StepState::Pending | StepState::Ready | StepState::Running
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
    Dismissed,
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
    #[serde(rename = "task.assigned")]
    TaskAssigned,
    #[serde(rename = "task.leased_out")]
    TaskLeasedOut,
    #[serde(rename = "task.completed")]
    TaskCompleted,
    #[serde(rename = "task.failed")]
    TaskFailed,
    #[serde(rename = "task.reserved")]
    TaskReserved,
    #[serde(rename = "task.updated")]
    TaskUpdated,
    #[serde(rename = "task.hook")]
    TaskHook,
    #[serde(rename = "schedule.created")]
    ScheduleCreated,
    #[serde(rename = "schedule.fired")]
    ScheduleFired,
    #[serde(rename = "schedule.skipped")]
    ScheduleSkipped,
    #[serde(rename = "schedule.paused")]
    SchedulePaused,
    #[serde(rename = "schedule.resumed")]
    ScheduleResumed,
    #[serde(rename = "schedule.removed")]
    ScheduleRemoved,
    #[serde(rename = "schedule.updated")]
    ScheduleUpdated,
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
    #[serde(rename = "workflow.applied")]
    WorkflowApplied,
    #[serde(rename = "workflow.removed")]
    WorkflowRemoved,
    #[serde(rename = "workflow.enabled")]
    WorkflowEnabled,
    #[serde(rename = "workflow.disabled")]
    WorkflowDisabled,
    #[serde(rename = "workflow.run.waiting")]
    RunWaiting,
    #[serde(rename = "workflow.run.started")]
    RunStarted,
    #[serde(rename = "workflow.run.completed")]
    RunCompleted,
    #[serde(rename = "workflow.run.failed")]
    RunFailed,
    #[serde(rename = "workflow.run.canceled")]
    RunCanceled,
    #[serde(rename = "workflow.step.started")]
    StepStarted,
    #[serde(rename = "workflow.step.completed")]
    StepCompleted,
    #[serde(rename = "workflow.step.failed")]
    StepFailed,
    #[serde(rename = "workflow.step.canceled")]
    StepCanceled,
    #[serde(rename = "artifact.created")]
    ArtifactCreated,
}

impl EventKind {
    /// Wire name (the serde rename). A plain match: this runs per event
    /// per SSE subscriber, so it must not allocate.
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::ServerStarted => "server.started",
            EventKind::AgentCreated => "agent.created",
            EventKind::AgentRemoved => "agent.removed",
            EventKind::AgentStateChange => "agent.state",
            EventKind::AgentOutput => "agent.output",
            EventKind::TaskCreated => "task.created",
            EventKind::TaskStatus => "task.status",
            EventKind::TaskAssigned => "task.assigned",
            EventKind::TaskLeasedOut => "task.leased_out",
            EventKind::TaskCompleted => "task.completed",
            EventKind::TaskFailed => "task.failed",
            EventKind::TaskReserved => "task.reserved",
            EventKind::TaskUpdated => "task.updated",
            EventKind::TaskHook => "task.hook",
            EventKind::ScheduleCreated => "schedule.created",
            EventKind::ScheduleFired => "schedule.fired",
            EventKind::ScheduleSkipped => "schedule.skipped",
            EventKind::SchedulePaused => "schedule.paused",
            EventKind::ScheduleResumed => "schedule.resumed",
            EventKind::ScheduleRemoved => "schedule.removed",
            EventKind::ScheduleUpdated => "schedule.updated",
            EventKind::MessageCreated => "message.created",
            EventKind::MessageStatusChange => "message.status",
            EventKind::ApprovalExpired => "approval.expired",
            EventKind::MachineState => "machine.state",
            EventKind::NodeRegistered => "node.registered",
            EventKind::NodeDisconnected => "node.disconnected",
            EventKind::WorkflowApplied => "workflow.applied",
            EventKind::WorkflowRemoved => "workflow.removed",
            EventKind::WorkflowEnabled => "workflow.enabled",
            EventKind::WorkflowDisabled => "workflow.disabled",
            EventKind::RunWaiting => "workflow.run.waiting",
            EventKind::RunStarted => "workflow.run.started",
            EventKind::RunCompleted => "workflow.run.completed",
            EventKind::RunFailed => "workflow.run.failed",
            EventKind::RunCanceled => "workflow.run.canceled",
            EventKind::StepStarted => "workflow.step.started",
            EventKind::StepCompleted => "workflow.step.completed",
            EventKind::StepFailed => "workflow.step.failed",
            EventKind::StepCanceled => "workflow.step.canceled",
            EventKind::ArtifactCreated => "artifact.created",
        }
    }
}
