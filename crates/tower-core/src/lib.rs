//! tower core types: agents, tasks, messages, events, machines (DESIGN.md §5).

pub mod error;
pub mod ids;
pub mod objects;
pub mod states;

pub use error::{ErrorCode, TowerError};
pub use ids::{AgentId, EventSeq, MachineId, MessageId, TaskId};
pub use objects::{Agent, Artifact, Event, Machine, Message, Task};
pub use states::{
    AgentState, DesiredState, EventKind, MessageKind, MessageStatus, Part, PartyKind, Permissions,
    TaskState,
};

/// Generate a new ULID string.
pub fn new_id() -> String {
    ulid::Ulid::new().to_string()
}

/// Current unix epoch milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
