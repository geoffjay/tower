//! What the page shows, computed from a snapshot + the metrics cache at
//! `now`. Pure: no Topcoat, no I/O — the view only formats these.

use std::collections::HashMap;

use tower_core::{Agent, AgentState, Event, EventKind, Machine, QueueCounts, Task};

use crate::layout;
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
    pub state: AgentState,
    pub x: f64,
    pub y: f64,
    pub r: f64,
    /// Brightness (health), 0.35–1.
    pub health: f64,
    pub attention: Attention,
    pub activity: u32,
    /// CSS drift animation: duration and (negative) phase, seconds.
    pub drift_s: u32,
    pub drift_phase_s: u32,
}

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
    /// Labeled only when there is more than one machine.
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

pub fn cloud(snap: &Snapshot, metrics: &Metrics, now: i64) -> Cloud {
    let machines = ordered_machines(&snap.machines);
    // group agents by machine; unknown machines get their own cluster
    let mut order: Vec<String> = machines.iter().map(|m| m.id.0.clone()).collect();
    let mut groups: HashMap<&str, Vec<&Agent>> = HashMap::new();
    for a in &snap.agents {
        if !order.contains(&a.machine_id.0) {
            order.push(a.machine_id.0.clone());
        }
        groups.entry(a.machine_id.0.as_str()).or_default().push(a);
    }
    for g in groups.values_mut() {
        g.sort_by(|a, b| (a.created_at, &a.id.0).cmp(&(b.created_at, &b.id.0)));
    }
    let sizes: Vec<usize> = order
        .iter()
        .map(|id| groups.get(id.as_str()).map_or(0, Vec::len))
        .collect();
    let (centers, positions) = layout::layout(&sizes);

    let machine_name = |id: &str| {
        snap.machines
            .iter()
            .find(|m| m.id.0 == id)
            .map_or_else(|| id.to_string(), |m| m.name.clone())
    };

    let mut points = Vec::with_capacity(snap.agents.len());
    for (ci, mid) in order.iter().enumerate() {
        let Some(agents) = groups.get(mid.as_str()) else {
            continue;
        };
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
                state: a.state,
                x: pos.x,
                y: pos.y,
                r: metrics::radius(m.activity),
                health,
                attention: attention(snap, a, &m),
                activity: m.activity,
                drift_s: 9 + h % 7,
                drift_phase_s: (h / 7) % 15,
            });
        }
    }

    let clusters = if order.len() > 1 {
        order
            .iter()
            .zip(&centers)
            .zip(&positions)
            .map(|((id, c), pts)| Cluster {
                name: machine_name(id),
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
            .chain(clusters.iter().map(|c| layout::Pos { x: c.x, y: c.y })),
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
        clusters,
        queue: QueueCounts::count(&snap.open_tasks, |id| {
            snap.agents.iter().find(|a| &a.id == id).map(|a| a.state)
        }),
        inbox: snap.pending_from_agents.len(),
        machines: machines
            .iter()
            .map(|m| Chip {
                name: m.name.clone(),
                online: m.status == "online",
                agents: groups.get(m.id.0.as_str()).map_or(0, Vec::len),
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
