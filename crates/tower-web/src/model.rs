//! What the page shows, computed from a snapshot + the metrics cache at
//! `now`. Pure: no Topcoat, no I/O — the view only formats these.

use std::collections::HashMap;

use tower_core::{Agent, AgentState, Event, EventKind, Machine, QueueCounts, Task};

use crate::layout::{self, Pos};
use crate::metrics::{self, AgentMetrics, BUCKETS, HealthInputs, Metrics};
use crate::source::Snapshot;

/// Halo channel (S4.B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attention {
    /// Amber pulse: blocked, or a pending question/approval — needs you.
    Needs,
    /// Red steady ring: a fault in the last 15 min.
    Fault,
    None,
}

#[derive(Debug, Clone)]
pub struct Point {
    pub id: String,
    pub name: String,
    /// Display name: truncated so neighbors' labels stay readable; the
    /// full name is in the tooltip and panel.
    pub label: String,
    pub state: AgentState,
    pub x: f64,
    pub y: f64,
    /// The stack root this point binds to (D§12.1 spokes).
    pub root_x: f64,
    pub root_y: f64,
    pub r: f64,
    /// Brightness (health), 0.35–1.
    pub health: f64,
    pub attention: Attention,
    pub activity: u32,
    /// CSS drift animation: duration and (negative) phase, seconds.
    pub drift_s: u32,
    pub drift_phase_s: u32,
}

/// A stack's root node (D§12.1): every point of the stack binds to it
/// with a spoke line. The stack comes from the agent-name convention
/// (`<stack>-<role>`); tower stores no stack record.
#[derive(Debug, Clone)]
pub struct Root {
    pub id: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
    /// The machine node this root links to (D§12.1).
    pub machine_x: f64,
    pub machine_y: f64,
    /// Points bound to this root, in draw order.
    pub agents: usize,
}

/// A machine's node (D§12.1): hollow, colored by connection status,
/// linked from the roots of the stacks that run on it. Agents never
/// bind to it directly.
#[derive(Debug, Clone)]
pub struct MachineNode {
    pub id: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
    /// `machine.status == "online"`.
    pub online: bool,
    /// Points on this machine, for the tooltip.
    pub agents: usize,
}

/// Cluster label: the stack-name text above each cluster of points.
#[derive(Debug, Clone)]
pub struct Cluster {
    pub name: String,
    /// Label position: centered above the cluster's topmost point.
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone)]
pub struct Chip {
    pub name: String,
    pub online: bool,
    pub agents: usize,
}

#[derive(Debug, Clone)]
pub struct RibbonItem {
    pub seq: i64,
    pub age: String,
    pub kind: &'static str,
    pub who: String,
}

#[derive(Debug, Clone)]
pub struct Cloud {
    pub points: Vec<Point>,
    /// SVG viewBox framing the points.
    pub view_box: String,
    /// One per stack, in cluster order — the central nodes the points
    /// bind to (D§12.1 spokes).
    pub roots: Vec<Root>,
    /// One per machine, in display order — hollow status-colored nodes
    /// the roots link to (D§12.1).
    pub machine_nodes: Vec<MachineNode>,
    /// Stack labels, shown when there is more than one cluster.
    pub clusters: Vec<Cluster>,
    pub queue: QueueCounts,
    /// Pending questions/approvals from agents.
    pub inbox: usize,
    /// Names of the agents that need the operator, by name.
    pub needs_you: Vec<String>,
    pub machines: Vec<Chip>,
    /// Newest first.
    pub ribbon: Vec<RibbonItem>,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub title: String,
    pub state: String,
    /// Seconds until the lease expires; `None` while held (input-required)
    /// or unleased.
    pub lease_left_s: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Panel {
    pub name: String,
    pub kind: String,
    pub machine: String,
    pub state: AgentState,
    pub attention: Attention,
    pub health: f64,
    pub job: Option<Job>,
    pub activity: u32,
    pub msgs_per_min: f64,
    pub series: [u32; BUCKETS],
    pub snippet: Option<String>,
}

/// A stack root's panel (D§12.1): roll-up of the stack's agents — no
/// per-agent depth beyond the names, which stay clickable in spirit
/// (selecting the point opens the agent panel).
#[derive(Debug, Clone)]
pub struct StackPanel {
    pub name: String,
    pub machine: String,
    pub agents: usize,
    /// `(state, count)`, the D§12.1 state set, only non-zero states.
    pub states: Vec<(AgentState, usize)>,
    /// Events across the stack's agents in the window.
    pub activity: u32,
    /// Events per 10 s bucket, summed across the stack's agents.
    pub series: [u32; BUCKETS],
    /// Messages to/from the stack's agents per minute over 5 min.
    pub msgs_per_min: f64,
    /// The stack's agent names, display order.
    pub agent_names: Vec<String>,
}

/// A machine node's panel (D§12.1): the machine row plus a roll-up of
/// the agents on it.
#[derive(Debug, Clone)]
pub struct MachinePanel {
    pub name: String,
    /// `coordinator` / `node`; `None` without a machine row.
    pub role: Option<String>,
    pub address: Option<String>,
    /// The row's status (`online`, `offline`, ...); `unknown` without one.
    pub status: String,
    /// `status == "online"` — the node's color, and the heading dot's.
    pub online: bool,
    /// Age of the last heartbeat; `None` if never seen.
    pub last_seen: Option<String>,
    pub agents: usize,
    /// Stacks running on it, first-seen order (the cloud's cluster order).
    pub stacks: Vec<String>,
    /// `(state, count)`, the D§12.1 state set, only non-zero states.
    pub states: Vec<(AgentState, usize)>,
}

/// What the panel shows, per selection kind (D§12.1): `agent:<id>`,
/// `stack:<machine>/<stack>`, `machine:<machine>` — the page parses the
/// selection value.
#[derive(Debug, Clone)]
pub enum PanelKind {
    Agent(Panel),
    Stack(StackPanel),
    Machine(MachinePanel),
}

/// Machines in display order: the coordinator first, then by name.
fn ordered_machines(machines: &[Machine]) -> Vec<&Machine> {
    let mut m: Vec<&Machine> = machines.iter().collect();
    m.sort_by(|a, b| (a.role != "coordinator", &a.name).cmp(&(b.role != "coordinator", &b.name)));
    m
}

fn owned_job<'a>(tasks: &'a [Task], agent: &Agent) -> Option<&'a Task> {
    tasks
        .iter()
        .find(|t| t.owner_id.as_ref() == Some(&agent.id) && !t.state.is_terminal())
}

fn pending_from(snap: &Snapshot, agent: &Agent) -> bool {
    snap.pending_from_agents
        .iter()
        .any(|m| m.from_id == agent.id.0 || m.from_id == agent.name)
}

fn lease_left(job: Option<&Task>, now: i64) -> Option<f64> {
    let job = job?;
    let exp = job.lease_expires_at?;
    Some((exp - now) as f64 / (job.lease_s.max(1) * 1000) as f64)
}

/// Label shown under a point: names longer than `LABEL_MAX` chars become
/// `prefix…` so one long name can't crowd its neighbors (D§12.1). The
/// full name stays in the tooltip and the panel.
const LABEL_MAX: usize = 12;

fn display_label(name: &str) -> String {
    if name.chars().count() <= LABEL_MAX {
        name.to_owned()
    } else {
        let cut: String = name.chars().take(LABEL_MAX - 1).collect();
        format!("{cut}…")
    }
}

fn attention(snap: &Snapshot, agent: &Agent, m: &AgentMetrics) -> Attention {
    if agent.state == AgentState::Blocked || pending_from(snap, agent) {
        Attention::Needs
    } else if m.faults > 0 {
        Attention::Fault
    } else {
        Attention::None
    }
}

/// Stable small hash of an id, for the drift animation's variety.
fn id_hash(id: &str) -> u32 {
    id.bytes().fold(2_166_136_261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16_777_619)
    })
}

/// The stack of an agent name: `<stack>-<role>` — the first `-` wins
/// (the operator-skills convention); a name with no `-` is its own
/// stack. Tower stores no stack record (D§12.1).
pub fn stack_of(name: &str) -> &str {
    name.split_once('-').map_or(name, |(stack, _)| stack)
}

pub fn cloud(snap: &Snapshot, metrics: &Metrics, now: i64) -> Cloud {
    let machines = ordered_machines(&snap.machines);
    // cluster agents by (machine, stack): machines in display order,
    // stacks in first-seen order; agents on unknown machines still
    // cluster under that machine id
    let machine_of = |id: &str| {
        machines
            .iter()
            .position(|m| m.id.0 == id)
            .unwrap_or(usize::MAX)
    };
    let mut keys: Vec<(&str, &str)> = Vec::new(); // (machine id, stack)
    let mut groups: HashMap<(&str, &str), Vec<&Agent>> = HashMap::new();
    for a in &snap.agents {
        let key = (a.machine_id.0.as_str(), stack_of(&a.name));
        if !keys.contains(&key) {
            keys.push(key);
        }
        groups.entry(key).or_default().push(a);
    }
    keys.sort_by_key(|&(m, _)| machine_of(m)); // stable: keeps first-seen stack order
    for g in groups.values_mut() {
        g.sort_by(|a, b| (a.created_at, &a.id.0).cmp(&(b.created_at, &b.id.0)));
    }
    let sizes: Vec<usize> = keys
        .iter()
        .map(|k| groups.get(k).map_or(0, Vec::len))
        .collect();
    let (centers, positions) = layout::layout(&sizes);

    let machine_name = |id: &str| {
        snap.machines
            .iter()
            .find(|m| m.id.0 == id)
            .map_or_else(|| id.to_string(), |m| m.name.clone())
    };

    let mut points = Vec::with_capacity(snap.agents.len());
    for (ci, &(mid, stack)) in keys.iter().enumerate() {
        let agents = &groups[&(mid, stack)];
        for (a, pos) in agents.iter().zip(&positions[ci]) {
            let m = metrics.agent(&a.id.0, &a.name, now);
            let job = owned_job(&snap.open_tasks, a);
            let health = metrics::health(
                HealthInputs {
                    state: a.state,
                    lease_left: lease_left(job, now),
                },
                &m,
                now,
            );
            let h = id_hash(&a.id.0);
            points.push(Point {
                id: a.id.0.clone(),
                name: a.name.clone(),
                label: display_label(&a.name),
                state: a.state,
                x: pos.x,
                y: pos.y,
                root_x: centers[ci].x,
                root_y: centers[ci].y,
                r: metrics::radius(m.activity),
                health,
                attention: attention(snap, a, &m),
                activity: m.activity,
                // slow drift (D§12.1): 24-35 s per cycle, small offsets
                drift_s: 24 + h % 12,
                drift_phase_s: (h / 12) % 35,
            });
        }
    }

    // machine nodes: one per machine with clusters on it (display
    // order), at the centroid of its clusters' root centers — the roots
    // link to it (D§12.1); its color is the connection status
    let machine_ids: Vec<&str> = {
        let mut ids: Vec<&str> = keys.iter().map(|(m, _)| *m).collect();
        ids.dedup();
        ids
    };
    let machine_pos: HashMap<&str, Pos> = machine_ids
        .iter()
        .map(|&mid| {
            let own: Vec<usize> = keys
                .iter()
                .enumerate()
                .filter(|&(_, (m, _))| m == &mid)
                .map(|(i, _)| i)
                .collect();
            // centroid of the machine's cluster centers
            let n = own.len() as f64;
            (
                mid,
                Pos {
                    x: own.iter().map(|&i| centers[i].x).sum::<f64>() / n,
                    y: own.iter().map(|&i| centers[i].y).sum::<f64>() / n,
                },
            )
        })
        .collect();
    let machine_nodes: Vec<MachineNode> = machine_ids
        .iter()
        .map(|&mid| {
            let pos = machine_pos[mid];
            MachineNode {
                id: mid.to_string(),
                name: machine_name(mid),
                x: pos.x,
                y: pos.y,
                online: snap
                    .machines
                    .iter()
                    .any(|m| m.id.0 == mid && m.status == "online"),
                agents: snap.agents.iter().filter(|a| a.machine_id.0 == mid).count(),
            }
        })
        .collect();

    // stack roots: the central nodes the points bind to, each linked to
    // its machine's node (D§12.1)
    let roots: Vec<Root> = keys
        .iter()
        .zip(&centers)
        .map(|((mid, stack), c)| {
            let m = machine_pos[mid];
            Root {
                id: format!("{mid}/{stack}"),
                name: (*stack).to_string(),
                x: c.x,
                y: c.y,
                machine_x: m.x,
                machine_y: m.y,
                agents: groups.get(&(*mid, *stack)).map_or(0, Vec::len),
            }
        })
        .collect();

    let cluster_labels = if keys.len() > 1 {
        keys.iter()
            .zip(&centers)
            .zip(&positions)
            .map(|(((_, stack), c), pts)| Cluster {
                name: (*stack).to_string(),
                x: c.x,
                y: pts.iter().map(|p| p.y).fold(c.y, f64::min) - 32.0,
            })
            .collect()
    } else {
        Vec::new()
    };
    let (vx, vy, vw, vh) = layout::frame(
        positions
            .iter()
            .flatten()
            .copied()
            .chain(roots.iter().map(|r| layout::Pos { x: r.x, y: r.y }))
            .chain(machine_nodes.iter().map(|m| layout::Pos { x: m.x, y: m.y }))
            .chain(
                cluster_labels
                    .iter()
                    .map(|c| layout::Pos { x: c.x, y: c.y }),
            ),
    );

    let names: HashMap<&str, &str> = snap
        .agents
        .iter()
        .map(|a| (a.id.0.as_str(), a.name.as_str()))
        .collect();
    let ribbon = metrics
        .ribbon()
        .rev()
        .map(|e| RibbonItem {
            seq: e.seq,
            age: age(now - e.ts),
            kind: e.kind.as_str(),
            who: who(e, &names),
        })
        .collect();

    Cloud {
        needs_you: {
            let mut n: Vec<String> = points
                .iter()
                .filter(|p| p.attention == Attention::Needs)
                .map(|p| p.name.clone())
                .collect();
            n.sort();
            n
        },
        points,
        view_box: format!("{vx:.0} {vy:.0} {vw:.0} {vh:.0}"),
        roots,
        machine_nodes,
        clusters: cluster_labels,
        queue: QueueCounts::count(&snap.open_tasks, |id| {
            snap.agents.iter().find(|a| &a.id == id).map(|a| a.state)
        }),
        inbox: snap.pending_from_agents.len(),
        machines: machines
            .iter()
            .map(|m| Chip {
                name: m.name.clone(),
                online: m.status == "online",
                agents: snap.agents.iter().filter(|a| a.machine_id == m.id).count(),
            })
            .collect(),
        ribbon,
    }
}

pub fn panel(snap: &Snapshot, metrics: &Metrics, agent_id: &str, now: i64) -> Option<Panel> {
    let a = snap.agents.iter().find(|a| a.id.0 == agent_id)?;
    let m = metrics.agent(&a.id.0, &a.name, now);
    let job = owned_job(&snap.open_tasks, a);
    Some(Panel {
        name: a.name.clone(),
        kind: a.kind.clone(),
        machine: snap
            .machines
            .iter()
            .find(|mm| mm.id == a.machine_id)
            .map_or_else(|| a.machine_id.0.clone(), |mm| mm.name.clone()),
        state: a.state,
        attention: attention(snap, a, &m),
        health: metrics::health(
            HealthInputs {
                state: a.state,
                lease_left: lease_left(job, now),
            },
            &m,
            now,
        ),
        job: job.map(|t| Job {
            title: t.title.clone(),
            state: serde_json::to_value(t.state)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default(),
            lease_left_s: match t.state {
                tower_core::TaskState::InputRequired => None,
                _ => t
                    .lease_expires_at
                    .map(|exp| ((exp - now) as f64 / 1000.0).ceil().max(0.0) as i64),
            },
        }),
        activity: m.activity,
        msgs_per_min: m.msgs_per_min,
        series: m.series,
        snippet: m.snippet,
    })
}

/// A stack root's panel: agents of the (machine, stack) pair, rolled up
/// per D§12.1. `machine_id/stack` mirrors [Root::id], so the selection
/// value `stack:{root.id}` routes here. Unknown pair → `None`, the same
/// closed-panel contract as [panel] for an unknown agent.
pub fn stack_panel(
    snap: &Snapshot,
    metrics: &Metrics,
    machine_id: &str,
    stack: &str,
    now: i64,
) -> Option<StackPanel> {
    let agents: Vec<&Agent> = snap
        .agents
        .iter()
        .filter(|a| a.machine_id.0 == machine_id && stack_of(&a.name) == stack)
        .collect();
    if agents.is_empty() {
        return None;
    }
    let states = state_counts(&agents);
    let mut series = [0u32; BUCKETS];
    let mut activity = 0;
    let mut msgs_per_min = 0.0;
    let mut agent_names = Vec::with_capacity(agents.len());
    for a in &agents {
        let m = metrics.agent(&a.id.0, &a.name, now);
        activity += m.activity;
        msgs_per_min += m.msgs_per_min;
        for (dst, src) in series.iter_mut().zip(m.series.iter()) {
            *dst += src;
        }
        agent_names.push(a.name.clone());
    }
    Some(StackPanel {
        name: stack.to_string(),
        machine: snap
            .machines
            .iter()
            .find(|mm| mm.id.0 == machine_id)
            .map_or_else(|| machine_id.to_string(), |mm| mm.name.clone()),
        agents: agents.len(),
        states,
        activity,
        series,
        msgs_per_min,
        agent_names,
    })
}

/// A machine node's panel: the machine row and its agents, rolled up per
/// D§12.1. `machine_id` mirrors [MachineNode::id], so the selection value
/// `machine:{node.id}` routes here. Agents on a machine with no row still
/// get a panel (the cloud draws their node too); an id with neither a row
/// nor agents → `None`, the closed-panel contract.
pub fn machine_panel(snap: &Snapshot, machine_id: &str, now: i64) -> Option<MachinePanel> {
    let row = snap.machines.iter().find(|m| m.id.0 == machine_id);
    let agents: Vec<&Agent> = snap
        .agents
        .iter()
        .filter(|a| a.machine_id.0 == machine_id)
        .collect();
    if row.is_none() && agents.is_empty() {
        return None;
    }
    let mut stacks: Vec<String> = Vec::new();
    for a in &agents {
        let s = stack_of(&a.name);
        if !stacks.iter().any(|x| x == s) {
            stacks.push(s.to_string());
        }
    }
    let status = row.map_or("unknown", |m| m.status.as_str());
    Some(MachinePanel {
        name: row.map_or_else(|| machine_id.to_string(), |m| m.name.clone()),
        role: row.map(|m| m.role.clone()),
        address: row.and_then(|m| m.address.clone()),
        status: status.to_string(),
        online: status == "online",
        last_seen: row.and_then(|m| m.last_seen_at).map(|t| age(now - t)),
        agents: agents.len(),
        stacks,
        states: state_counts(&agents),
    })
}

/// Per-state counts in the D§12.1 legend order, non-zero only.
fn state_counts(agents: &[&Agent]) -> Vec<(AgentState, usize)> {
    [
        AgentState::Working,
        AgentState::Blocked,
        AgentState::Idle,
        AgentState::Done,
        AgentState::Dead,
        AgentState::Launching,
        AgentState::Unknown,
    ]
    .into_iter()
    .filter_map(|s| {
        let n = agents.iter().filter(|a| a.state == s).count();
        (n > 0).then_some((s, n))
    })
    .collect()
}

/// Compact age: `4s`, `12m`, `3h`, `2d`.
pub fn age(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

/// The agent an event concerns, by name when known.
fn who(e: &Event, names: &HashMap<&str, &str>) -> String {
    let id = if e.subject_type.as_deref() == Some("agent") {
        e.subject_id.as_deref()
    } else {
        ["owner_id", "prior_owner", "agent_id"]
            .iter()
            .find_map(|k| e.payload.get(*k).and_then(|v| v.as_str()))
    };
    if let Some(id) = id {
        if let Some(n) = names.get(id) {
            return n.to_string();
        }
        // a removed agent: its name if the event carries it, else a short id
        return e.payload.get("name").and_then(|v| v.as_str()).map_or_else(
            || {
                let tail: Vec<char> = id.chars().rev().take(6).collect();
                format!("…{}", tail.into_iter().rev().collect::<String>())
            },
            str::to_string,
        );
    }
    if e.kind == EventKind::MessageCreated {
        let from = e.payload["from"]["id"].as_str().unwrap_or("?");
        let to = e.payload["to"]["id"].as_str().unwrap_or("?");
        let n = |x: &str| names.get(x).map_or(x, |n| n).to_string();
        return format!("{} → {}", n(from), n(to));
    }
    e.payload
        .get("title")
        .and_then(|v| v.as_str())
        .or(e.subject_id.as_deref())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use tower_core::{
        Agent, AgentId, AgentState, DesiredState, Event, EventKind, Machine, MachineId, Permissions,
    };

    use super::{machine_panel, stack_of, stack_panel};
    use crate::metrics::Metrics;
    use crate::source::Snapshot;

    const NOW: i64 = 1_790_000_000_000;

    fn agent(name: &str, machine: &str, state: AgentState) -> Agent {
        Agent {
            id: AgentId(format!("A_{name}")),
            name: name.into(),
            kind: "pi".into(),
            machine_id: MachineId(machine.into()),
            pane_id: None,
            workdir: None,
            worktree: None,
            state,
            desired_state: DesiredState::Running,
            permissions: Permissions::Default,
            adopted: false,
            config: serde_json::json!({}),
            model: None,
            args: vec![],
            created_at: NOW - 3_600_000,
            updated_at: NOW,
        }
    }

    fn machine(id: &str) -> Machine {
        Machine {
            id: MachineId(id.into()),
            name: format!("{id}-host"),
            role: "node".into(),
            address: None,
            status: "online".into(),
            last_seen_at: Some(NOW),
            created_at: NOW - 86_400_000,
        }
    }

    /// `n` output events for one agent, 10 s apart, ending at NOW.
    fn activity(metrics: &mut Metrics, name: &str, n: i64) {
        for i in 0..n {
            metrics.apply(&Event {
                seq: i,
                ts: NOW - i * 10_000,
                kind: EventKind::AgentOutput,
                subject_type: Some("agent".into()),
                subject_id: Some(format!("A_{name}")),
                payload: serde_json::json!({}),
            });
        }
    }

    #[test]
    fn stack_is_the_name_before_the_first_dash() {
        // `<stack>-<role>`: the first `-` wins; no dash → own stack
        assert_eq!(stack_of("s1-lead"), "s1");
        assert_eq!(stack_of("s2-api-2"), "s2");
        assert_eq!(stack_of("-lead"), "");
        assert_eq!(stack_of("pi"), "pi");
        assert_eq!(stack_of(""), "");
    }

    #[test]
    fn stack_panel_rolls_up_only_its_machine_and_stack() {
        let snap = Snapshot {
            machines: vec![machine("m1"), machine("m2")],
            agents: vec![
                agent("s1-lead", "m1", AgentState::Working),
                agent("s1-web", "m1", AgentState::Blocked),
                agent("s1-api", "m1", AgentState::Working),
                // same stack name on another machine: a different root
                agent("s1-ops", "m2", AgentState::Dead),
                // another stack on the same machine
                agent("s2-lead", "m1", AgentState::Idle),
            ],
            ..Default::default()
        };
        let mut metrics = Metrics::default();
        activity(&mut metrics, "s1-lead", 3);
        activity(&mut metrics, "s1-web", 2);
        activity(&mut metrics, "s1-ops", 5); // m2: must not count
        activity(&mut metrics, "s2-lead", 7); // s2: must not count

        let p = stack_panel(&snap, &metrics, "m1", "s1", NOW).unwrap();
        assert_eq!(p.name, "s1");
        assert_eq!(p.machine, "m1-host", "machine display name, not id");
        assert_eq!(p.agents, 3);
        assert_eq!(p.agent_names, ["s1-lead", "s1-web", "s1-api"]);
        // legend order, non-zero only: no idle/dead from other roots
        assert_eq!(
            p.states,
            [(AgentState::Working, 2), (AgentState::Blocked, 1)]
        );
        // activity and the sparkline sum across the stack's agents only
        assert_eq!(p.activity, 5);
        assert_eq!(p.series.iter().sum::<u32>(), 5);
    }

    #[test]
    fn stack_panel_is_none_for_an_unknown_pair() {
        let snap = Snapshot {
            machines: vec![machine("m1")],
            agents: vec![agent("s1-lead", "m1", AgentState::Idle)],
            ..Default::default()
        };
        let metrics = Metrics::default();
        assert!(stack_panel(&snap, &metrics, "m1", "s9", NOW).is_none());
        assert!(stack_panel(&snap, &metrics, "m9", "s1", NOW).is_none());
    }

    #[test]
    fn stack_panel_names_an_unknown_machine_by_its_id() {
        // the agent's machine row is gone (removed node): still opens
        let snap = Snapshot {
            agents: vec![agent("pi", "gone", AgentState::Idle)],
            ..Default::default()
        };
        let p = stack_panel(&snap, &Metrics::default(), "gone", "pi", NOW).unwrap();
        assert_eq!(p.machine, "gone");
        assert_eq!(p.name, "pi", "a no-dash name is its own stack");
    }

    #[test]
    fn machine_panel_rolls_up_its_agents_and_stacks() {
        let mut m1 = machine("m1");
        m1.role = "coordinator".into();
        m1.address = Some("10.0.0.5:7300".into());
        // offline: the panel still opens, with a stale last-seen age
        m1.status = "offline".into();
        m1.last_seen_at = Some(NOW - 12 * 60_000);
        let snap = Snapshot {
            machines: vec![m1, machine("m2")],
            agents: vec![
                agent("s2-lead", "m1", AgentState::Idle),
                agent("s1-lead", "m1", AgentState::Working),
                agent("s2-web", "m1", AgentState::Working),
                agent("s1-ops", "m2", AgentState::Dead), // other machine
            ],
            ..Default::default()
        };
        let p = machine_panel(&snap, "m1", NOW).unwrap();
        assert_eq!(p.name, "m1-host", "display name, not id");
        assert_eq!(p.role.as_deref(), Some("coordinator"));
        assert_eq!(p.address.as_deref(), Some("10.0.0.5:7300"));
        assert_eq!(p.status, "offline");
        assert!(!p.online);
        assert_eq!(p.last_seen.as_deref(), Some("12m"));
        assert_eq!(p.agents, 3);
        // every stack on it, once each, first-seen (cluster) order
        assert_eq!(p.stacks, ["s2", "s1"]);
        // legend order, non-zero only: m2's dead agent is not here
        assert_eq!(p.states, [(AgentState::Working, 2), (AgentState::Idle, 1)]);
        assert!(machine_panel(&snap, "m2", NOW).unwrap().online);
    }

    #[test]
    fn machine_panel_opens_from_either_a_row_or_agents() {
        let snap = Snapshot {
            machines: vec![machine("m1")],
            agents: vec![agent("pi", "gone", AgentState::Idle)],
            ..Default::default()
        };
        // a row with no agents: opens, empty rollup
        let p = machine_panel(&snap, "m1", NOW).unwrap();
        assert_eq!((p.agents, p.stacks.len(), p.states.len()), (0, 0, 0));
        // agents whose machine row is gone: named by id, nothing known
        let p = machine_panel(&snap, "gone", NOW).unwrap();
        assert_eq!(p.name, "gone");
        assert_eq!((p.role, p.address, p.last_seen), (None, None, None));
        assert_eq!(p.status, "unknown");
        assert!(!p.online);
        assert_eq!(p.stacks, ["pi"]);
        // neither → closed panel
        assert!(machine_panel(&snap, "m9", NOW).is_none());
    }
}
