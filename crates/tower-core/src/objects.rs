//! Row objects (D§5-6) — serde shapes shared by server, clients, UIs.

use serde::{Deserialize, Serialize};

use crate::ids::{AgentId, MachineId, MessageId, ScheduleId, TaskId};
use crate::states::{
    AgentState, DesiredState, EventKind, MessageKind, MessageStatus, PartyKind, Permissions,
    TaskState,
};

/// An agent row: named seat bound to a harness instance (D§5.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub name: String,
    pub kind: String,
    pub machine_id: MachineId,
    pub pane_id: Option<String>,
    pub workdir: Option<String>,
    pub worktree: Option<String>,
    pub state: AgentState,
    pub desired_state: DesiredState,
    pub permissions: Permissions,
    pub adopted: bool,
    #[serde(default)]
    pub config: serde_json::Value,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A task row (D§5.2, §5.2.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<AgentId>,
    pub origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub state: TaskState,
    pub priority: i64,
    pub tags: Vec<String>,
    pub attempt_count: i64,
    pub max_attempts: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<i64>,
    /// Renewal window in seconds for assign/start/heartbeat (D§5.2.1).
    pub lease_s: i64,
    /// Reserved for this agent: delivered when it is available (D§5.2.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_agent_id: Option<AgentId>,
    /// The dispatcher holds the job until this time (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_before: Option<i64>,
    /// Schedule that materialized this job, and the occurrence it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<ScheduleId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A recurring job template (D§5.2.2): fires on `cron` in `timezone`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: ScheduleId,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub priority: i64,
    pub lease_s: i64,
    pub max_attempts: i64,
    /// Jobs are reserved for this agent; `None` → general queue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_agent_id: Option<AgentId>,
    pub cron: String,
    /// IANA zone name, fixed at creation.
    pub timezone: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A message row (D§5.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    pub from_kind: PartyKind,
    pub from_id: String,
    pub to_kind: PartyKind,
    pub to_id: String,
    pub kind: MessageKind,
    pub parts: Vec<crate::states::Part>,
    pub status: MessageStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responded_at: Option<i64>,
    pub created_at: i64,
}

/// An artifact row (D§6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub task_id: Option<TaskId>,
    pub name: String,
    pub media_type: Option<String>,
    pub content_path: String,
    pub size: i64,
    pub created_at: i64,
}

/// A machine row (D§5.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Machine {
    pub id: MachineId,
    pub name: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<i64>,
    pub created_at: i64,
}

/// An event row: envelope + payload (D§5.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub ts: i64,
    pub kind: EventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
    pub payload: serde_json::Value,
}
