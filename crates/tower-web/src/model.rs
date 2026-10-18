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
    use super::stack_of;

    #[test]
    fn stack_is_the_name_before_the_first_dash() {
        // `<stack>-<role>`: the first `-` wins; no dash → own stack
        assert_eq!(stack_of("s1-lead"), "s1");
        assert_eq!(stack_of("s2-api-2"), "s2");
        assert_eq!(stack_of("-lead"), "");
        assert_eq!(stack_of("pi"), "pi");
        assert_eq!(stack_of(""), "");
    }
}
