//! Row objects (D§5-6) — serde shapes shared by server, clients, UIs.

use serde::{Deserialize, Serialize};

use crate::ids::{
    AgentId, ArtifactId, MachineId, MessageId, RunId, ScheduleId, StepId, TaskId, WorkflowId,
};
use crate::states::{
    AgentState, DesiredState, EventKind, HookPhase, MessageKind, MessageStatus, PartyKind,
    Permissions, RunState, StepState, TaskState,
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
    /// Model the harness runs on (`--model <id>` at spawn, issue #4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Extra harness args the agent was spawned with (per-agent, not the
    /// per-kind `[harness.<kind>] args` defaults).
    #[serde(default)]
    pub args: Vec<String>,
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
    /// Lifecycle prompt typed into the harness before the job prompt, on
    /// every delivery (D§5.2.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_run: Option<String>,
    /// Lifecycle prompt typed after the owner closes the job (D§5.2.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_run: Option<String>,
    /// The lifecycle prompt in flight, if any (lease expiry pauses during
    /// `pre`; the owner counts as busy during `post`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<HookPhase>,
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
    /// Lifecycle prompts copied into every job it fires (D§5.2.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_run: Option<String>,
    /// Managed by this workflow's schedule trigger: a firing starts a run
    /// instead of creating a job (D§5.2.2, D§5.6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<WorkflowId>,
    /// The input object of the runs it starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_input: Option<serde_json::Value>,
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

/// An artifact row (D§6, D§5.6): text attached to a job by its owner. The
/// content is served separately (`GET /v1/artifacts/{id}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: ArtifactId,
    pub task_id: TaskId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    /// Content length in bytes.
    pub size: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<AgentId>,
    pub created_at: i64,
}

/// A workflow definition (D§5.6); the graph lives in its versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: WorkflowId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub enabled: bool,
    pub current_version: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One execution of one workflow version (D§5.6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: RunId,
    pub workflow_id: WorkflowId,
    pub workflow_name: String,
    pub version: i64,
    pub state: RunState,
    pub input: serde_json::Value,
    /// What started it: `{kind: manual|schedule|event, …}`.
    pub trigger: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
}

/// One execution of one node in a run (D§5.6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: StepId,
    pub run_id: RunId,
    pub node_id: String,
    pub visit: i64,
    pub state: StepState,
    /// Steps whose edges fired into this one.
    pub activated_by: Vec<StepId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// `{summary, data?}`, frozen when the step closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
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
