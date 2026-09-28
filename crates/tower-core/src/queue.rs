//! Open-job counts shown by every monitoring surface (TUI banner D§11,
//! web pool bar D§12.2) — one definition so they never disagree.

use crate::{AgentId, AgentState, Task, TaskState};

/// Open jobs by state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct QueueCounts {
    pub queued: usize,
    pub working: usize,
    /// `input-required`, or assigned/working under a `blocked` owner.
    pub blocked: usize,
}

impl QueueCounts {
    /// Count `tasks`; `agent_state` resolves an owner's current state.
    pub fn count<'a>(
        tasks: impl IntoIterator<Item = &'a Task>,
        agent_state: impl Fn(&AgentId) -> Option<AgentState>,
    ) -> Self {
        let mut c = Self::default();
        for t in tasks {
            let owner_blocked = t
                .owner_id
                .as_ref()
                .and_then(&agent_state)
                .is_some_and(|s| s == AgentState::Blocked);
            match t.state {
                TaskState::Queued => c.queued += 1,
                TaskState::InputRequired => c.blocked += 1,
                TaskState::Assigned | TaskState::Working if owner_blocked => c.blocked += 1,
                TaskState::Assigned | TaskState::Working => c.working += 1,
                _ => {}
            }
        }
        c
    }
}
