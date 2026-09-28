//! Client-side state + the event reducer (plan T1.2, D§11.1).
//!
//! `Store::apply` folds one `/v1/events` row into local state. It patches
//! what the payload carries (agent state) and otherwise names what to
//! refetch — refetches are idempotent, so events racing the snapshot or
//! replayed after a reconnect are harmless. View-scoped fetches (output,
//! history, task detail) are returned for every matching event; the app
//! drops those that don't concern the open view.

use std::collections::{HashMap, VecDeque};

use tower_core::{
    Agent, AgentId, AgentState, Event, EventKind, Machine, Message, QueueCounts, Schedule, Task,
    TaskId,
};

/// Events kept for the events view.
pub const EVENTS_CAP: usize = 2000;

/// Closed jobs kept client-side (the recent pane shows 10).
pub const CLOSED_KEEP: usize = 100;

/// Something to (re)load from `/v1`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Fetch {
    /// The startup / resync snapshot: every list below.
    All,
    Agents,
    Tasks,
    /// `GET /v1/tasks?since=` — lease renewals emit no event.
    TasksSince(i64),
    Task(TaskId),
    Schedules,
    Inbox,
    Machines,
    /// The agent's screen (`read?format=ansi`).
    Output(AgentId),
    /// Messages sent by or to the agent.
    History(AgentId),
    TaskDetail(TaskId),
}

/// A loaded result, applied by `Store::load`.
#[derive(Debug, Clone)]
pub enum Loaded {
    Agents(Vec<Agent>),
    /// Full replace.
    Tasks(Vec<Task>),
    /// Upsert.
    TasksDelta(Vec<Task>),
    Schedules(Vec<Schedule>),
    Inbox(Vec<Message>),
    Machines(Vec<Machine>),
    Output {
        agent: AgentId,
        text: String,
    },
    History {
        agent: AgentId,
        messages: Vec<Message>,
    },
    TaskDetail(Box<TaskDetail>),
}

#[derive(Debug, Clone)]
pub struct TaskDetail {
    pub task: Task,
    pub trail: Vec<Event>,
    pub messages: Vec<Message>,
}

#[derive(Debug, Default)]
pub struct Store {
    /// Sorted by name.
    pub agents: Vec<Agent>,
    pub tasks: HashMap<TaskId, Task>,
    pub schedules: Vec<Schedule>,
    /// Pending, addressed to me; newest first.
    pub inbox: Vec<Message>,
    pub machines: Vec<Machine>,
    pub events: VecDeque<Event>,
    /// Highest event seq applied (replays at or below it are skipped).
    pub seq: Option<i64>,
    pub output: Option<(AgentId, String)>,
    pub history: Option<(AgentId, Vec<Message>)>,
    pub task_detail: Option<TaskDetail>,
}

impl Store {
    /// Fold one event in; returns what to refetch.
    pub fn apply(&mut self, mut e: Event) -> Vec<Fetch> {
        if self.seq.is_some_and(|s| e.seq <= s) {
            return Vec::new();
        }
        self.seq = Some(e.seq);
        let subject = e.subject_id.clone();
        let fetches = match e.kind {
            EventKind::ServerStarted => vec![Fetch::All],
            EventKind::AgentCreated => vec![Fetch::Agents],
            EventKind::AgentRemoved => {
                if let Some(id) = &subject {
                    self.agents.retain(|a| a.id.0 != *id);
                }
                vec![Fetch::Agents]
            }
            EventKind::AgentStateChange => self.patch_agent_state(&e),
            EventKind::AgentOutput => match subject {
                Some(id) => vec![Fetch::Output(AgentId(id))],
                None => vec![],
            },
            EventKind::TaskCreated
            | EventKind::TaskStatus
            | EventKind::TaskAssigned
            | EventKind::TaskLeasedOut
            | EventKind::TaskCompleted
            | EventKind::TaskFailed
            | EventKind::TaskReserved => match subject {
                Some(id) => vec![Fetch::Task(id.clone().into()), Fetch::TaskDetail(id.into())],
                None => vec![Fetch::Tasks],
            },
            EventKind::ScheduleCreated
            | EventKind::ScheduleFired
            | EventKind::ScheduleSkipped
            | EventKind::SchedulePaused
            | EventKind::ScheduleResumed
            | EventKind::ScheduleRemoved => vec![Fetch::Schedules],
            EventKind::MessageCreated
            | EventKind::MessageStatusChange
            | EventKind::ApprovalExpired => {
                let mut f = vec![Fetch::Inbox];
                f.extend(self.history_fetch(&e));
                f
            }
            EventKind::MachineState | EventKind::NodeRegistered | EventKind::NodeDisconnected => {
                vec![Fetch::Machines]
            }
            _ => vec![],
        };
        // output payloads can be whole screens: keep only their size
        if e.kind == EventKind::AgentOutput {
            let bytes = e.payload["text"].as_str().map_or(0, str::len);
            e.payload = serde_json::json!({ "bytes": bytes });
        }
        self.events.push_back(e);
        while self.events.len() > EVENTS_CAP {
            self.events.pop_front();
        }
        fetches
    }

    fn patch_agent_state(&mut self, e: &Event) -> Vec<Fetch> {
        let to = e.payload["to"]
            .as_str()
            .and_then(|s| serde_json::from_value::<AgentState>(s.into()).ok());
        let agent = e
            .subject_id
            .as_deref()
            .and_then(|id| self.agents.iter_mut().find(|a| a.id.0 == id));
        match (agent, to) {
            (Some(a), Some(to)) => {
                a.state = to;
                a.updated_at = e.ts;
                Vec::new()
            }
            _ => vec![Fetch::Agents],
        }
    }

    /// Message events concern an agent's history when the agent is the
    /// subject (prompts) or a party (addresses are names or ids).
    fn history_fetch(&self, e: &Event) -> Option<Fetch> {
        if e.subject_type.as_deref() == Some("agent") {
            return e.subject_id.clone().map(|id| Fetch::History(AgentId(id)));
        }
        let parties = [&e.payload["from"]["id"], &e.payload["to"]["id"]];
        parties.iter().find_map(|p| {
            let p = p.as_str()?;
            self.agents
                .iter()
                .find(|a| a.name == p || a.id.0 == p)
                .map(|a| Fetch::History(a.id.clone()))
        })
    }

    pub fn load(&mut self, l: Loaded) {
        match l {
            Loaded::Agents(mut a) => {
                a.sort_by(|x, y| x.name.cmp(&y.name));
                self.agents = a;
            }
            Loaded::Tasks(t) => {
                self.tasks = t.into_iter().map(|t| (t.id.clone(), t)).collect();
                self.prune_closed();
            }
            Loaded::TasksDelta(t) => {
                for t in t {
                    self.upsert_task(t);
                }
                self.prune_closed();
            }
            Loaded::Schedules(s) => self.schedules = s,
            Loaded::Inbox(m) => self.inbox = m,
            Loaded::Machines(m) => self.machines = m,
            Loaded::Output { agent, text } => self.output = Some((agent, text)),
            Loaded::History { agent, messages } => self.history = Some((agent, messages)),
            Loaded::TaskDetail(d) => {
                self.upsert_task(d.task.clone());
                self.task_detail = Some(*d);
            }
        }
    }

    /// Keep every open job but only the latest `CLOSED_KEEP` closed ones:
    /// the job history grows forever server-side, a long-running TUI must not.
    fn prune_closed(&mut self) {
        let mut closed: Vec<(i64, TaskId)> = self
            .tasks
            .values()
            .filter(|t| t.state.is_terminal())
            .map(|t| (t.updated_at, t.id.clone()))
            .collect();
        if closed.len() <= CLOSED_KEEP {
            return;
        }
        closed.sort_unstable_by_key(|c| std::cmp::Reverse(c.0));
        for (_, id) in closed.drain(CLOSED_KEEP..) {
            self.tasks.remove(&id);
        }
    }

    fn upsert_task(&mut self, t: Task) {
        match self.tasks.get(&t.id) {
            // an older fetch landing after a newer one must not regress
            Some(cur) if cur.updated_at > t.updated_at => {}
            _ => {
                self.tasks.insert(t.id.clone(), t);
            }
        }
    }

    /// High-water mark for `GET /v1/tasks?since=`.
    pub fn tasks_updated_at(&self) -> i64 {
        self.tasks.values().map(|t| t.updated_at).max().unwrap_or(0)
    }

    pub fn agent(&self, id: &AgentId) -> Option<&Agent> {
        self.agents.iter().find(|a| &a.id == id)
    }

    pub fn agent_by_name(&self, name: &str) -> Option<&Agent> {
        self.agents
            .iter()
            .find(|a| a.name == name || a.id.0 == name)
    }

    /// Display name for an agent id (the id itself when unknown).
    pub fn name_of<'a>(&'a self, id: &'a AgentId) -> &'a str {
        self.agent(id).map_or(id.0.as_str(), |a| a.name.as_str())
    }

    pub fn machine_name<'a>(&'a self, id: &'a tower_core::MachineId) -> &'a str {
        self.machines
            .iter()
            .find(|m| &m.id == id)
            .map_or(id.0.as_str(), |m| m.name.as_str())
    }

    /// The agent's open job (one job per agent, D§5.2.2).
    pub fn current_task(&self, agent: &AgentId) -> Option<&Task> {
        self.tasks
            .values()
            .find(|t| t.owner_id.as_ref() == Some(agent) && !t.state.is_terminal())
    }

    pub fn queue_counts(&self) -> QueueCounts {
        QueueCounts::count(self.tasks.values(), |id| self.agent(id).map(|a| a.state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_core::TaskState;

    fn task(n: i64, state: TaskState) -> Task {
        serde_json::from_value(serde_json::json!({
            "id": format!("t{n}"), "origin": "cli", "title": "x", "state": state,
            "priority": 0, "tags": [], "attempt_count": 0, "max_attempts": 3,
            "lease_s": 60, "created_at": n, "updated_at": n,
        }))
        .unwrap()
    }

    #[test]
    fn closed_jobs_are_bounded_open_ones_never_dropped() {
        let mut s = Store::default();
        let mut all: Vec<Task> = (0..150).map(|n| task(n, TaskState::Completed)).collect();
        all.push(task(-1, TaskState::Working)); // oldest of all, but open
        s.load(Loaded::Tasks(all));
        assert_eq!(s.tasks.len(), CLOSED_KEEP + 1);
        assert!(s.tasks.contains_key(&TaskId("t-1".into())));
        assert!(s.tasks.contains_key(&TaskId("t149".into())));
        assert!(
            !s.tasks.contains_key(&TaskId("t49".into())),
            "oldest closed pruned"
        );
        // deltas prune too
        s.load(Loaded::TasksDelta(vec![task(200, TaskState::Failed)]));
        assert_eq!(s.tasks.len(), CLOSED_KEEP + 1);
        assert!(!s.tasks.contains_key(&TaskId("t50".into())));
    }
}
