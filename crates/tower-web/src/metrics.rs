//! Cloud metrics (plan S4.B, D§12.1): rolled up once per event as the
//! follower applies it, cached per agent in fixed-size 10 s buckets, read
//! at render time relative to `now`. Memory per agent is constant; agents
//! idle past the fault window are dropped.

use std::collections::{HashMap, VecDeque};

use tower_core::{AgentState, Event, EventKind};

pub const BUCKET_MS: i64 = 10_000;
/// 30 × 10 s = the 5-minute activity window.
pub const BUCKETS: usize = 30;
pub const ACTIVITY_WINDOW_MS: i64 = BUCKET_MS * BUCKETS as i64;
/// Faults (leased out, failed, expired approval) count this long.
pub const FAULT_WINDOW_MS: i64 = 15 * 60_000;
/// A working agent with no activity this long is "silent".
pub const SILENCE_MS: i64 = 2 * 60_000;
/// Events kept for the ribbon.
pub const RIBBON_LEN: usize = 10;
const SNIPPET_LINES: usize = 6;
const SNIPPET_LINE_CHARS: usize = 100;

/// Counts per 10 s bucket; a slot is reused when its bucket index moves on.
#[derive(Debug, Clone, Copy)]
struct Buckets {
    counts: [u32; BUCKETS],
    index: [i64; BUCKETS],
}

impl Default for Buckets {
    fn default() -> Self {
        Self {
            counts: [0; BUCKETS],
            index: [i64::MIN; BUCKETS],
        }
    }
}

impl Buckets {
    fn add(&mut self, ts: i64) {
        let b = ts.div_euclid(BUCKET_MS);
        let slot = b.rem_euclid(BUCKETS as i64) as usize;
        if self.index[slot] != b {
            self.index[slot] = b;
            self.counts[slot] = 0;
        }
        self.counts[slot] += 1;
    }

    /// Oldest → newest, the bucket holding `now` last.
    fn series(&self, now: i64) -> [u32; BUCKETS] {
        let newest = now.div_euclid(BUCKET_MS);
        let mut out = [0; BUCKETS];
        for (k, v) in out.iter_mut().enumerate() {
            let b = newest - (BUCKETS - 1 - k) as i64;
            let slot = b.rem_euclid(BUCKETS as i64) as usize;
            if self.index[slot] == b {
                *v = self.counts[slot];
            }
        }
        out
    }

    fn merge(&mut self, other: &Buckets) {
        for slot in 0..BUCKETS {
            if other.index[slot] == i64::MIN {
                continue;
            }
            if self.index[slot] == other.index[slot] {
                self.counts[slot] += other.counts[slot];
            } else if other.index[slot] > self.index[slot] {
                self.index[slot] = other.index[slot];
                self.counts[slot] = other.counts[slot];
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Stats {
    activity: Buckets,
    messages: Buckets,
    /// The two most recent fault timestamps.
    faults: [Option<i64>; 2],
    last_activity: Option<i64>,
    snippet: Option<(i64, String)>,
}

impl Stats {
    fn fault(&mut self, ts: i64) {
        self.faults = [Some(ts), self.faults[0]];
    }

    fn merge(&mut self, other: &Stats) {
        self.activity.merge(&other.activity);
        self.messages.merge(&other.messages);
        let mut f: Vec<i64> = self
            .faults
            .iter()
            .chain(other.faults.iter())
            .flatten()
            .copied()
            .collect();
        f.sort_unstable_by(|a, b| b.cmp(a));
        self.faults = [f.first().copied(), f.get(1).copied()];
        self.last_activity = self.last_activity.max(other.last_activity);
        if other.snippet.as_ref().map(|s| s.0) > self.snippet.as_ref().map(|s| s.0) {
            self.snippet = other.snippet.clone();
        }
    }
}

/// One agent's rolled-up metrics at `now`.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentMetrics {
    /// Events attributed in the last 5 min.
    pub activity: u32,
    /// Activity per 10 s bucket, oldest first (the sparkline).
    pub series: [u32; BUCKETS],
    /// Messages to/from the agent per minute over 5 min.
    pub msgs_per_min: f64,
    /// Faults in the last 15 min (0–2 are tracked).
    pub faults: u32,
    pub last_activity: Option<i64>,
    /// The tail of its latest screen output.
    pub snippet: Option<String>,
}

#[derive(Debug, Default)]
pub struct Metrics {
    /// Keyed by agent id or name: message addresses and some payloads name
    /// an agent either way; `agent()` merges both keys.
    agents: HashMap<String, Stats>,
    ribbon: VecDeque<Event>,
}

impl Metrics {
    /// Fold one event in.
    pub fn apply(&mut self, e: &Event) {
        if e.kind == EventKind::AgentRemoved {
            if let Some(id) = &e.subject_id {
                self.agents.remove(id);
            }
            if let Some(name) = e.payload.get("name").and_then(|v| v.as_str()) {
                self.agents.remove(name);
            }
        }
        if e.kind != EventKind::AgentOutput {
            if self.ribbon.len() == RIBBON_LEN {
                self.ribbon.pop_front();
            }
            self.ribbon.push_back(e.clone());
        }
        if e.kind == EventKind::AgentRemoved {
            return;
        }

        for key in attributed(e) {
            let s = self.agents.entry(key.to_string()).or_default();
            s.activity.add(e.ts);
            s.last_activity = s.last_activity.max(Some(e.ts));
            match e.kind {
                EventKind::MessageCreated => s.messages.add(e.ts),
                EventKind::AgentOutput => {
                    if let Some(text) = e.payload.get("text").and_then(|v| v.as_str())
                        && let Some(tail) = snippet(text)
                    {
                        s.snippet = Some((e.ts, tail));
                    }
                }
                _ => {}
            }
        }
        if let Some(key) = fault_owner(e) {
            self.agents.entry(key.to_string()).or_default().fault(e.ts);
        }
    }

    /// Drop agents with nothing left in any window.
    pub fn prune(&mut self, now: i64) {
        self.agents.retain(|_, s| {
            s.last_activity.is_some_and(|t| now - t < FAULT_WINDOW_MS)
                || s.faults[0].is_some_and(|t| now - t < FAULT_WINDOW_MS)
        });
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.agents.len()
    }

    /// Newest last; output chunks excluded.
    pub fn ribbon(&self) -> impl DoubleEndedIterator<Item = &Event> {
        self.ribbon.iter()
    }

    pub fn agent(&self, id: &str, name: &str, now: i64) -> AgentMetrics {
        let mut s = self.agents.get(id).cloned().unwrap_or_default();
        if name != id
            && let Some(other) = self.agents.get(name)
        {
            s.merge(other);
        }
        let series = s.activity.series(now);
        let msgs: u32 = s.messages.series(now).iter().sum();
        AgentMetrics {
            activity: series.iter().sum(),
            series,
            msgs_per_min: f64::from(msgs) / (ACTIVITY_WINDOW_MS as f64 / 60_000.0),
            faults: s
                .faults
                .iter()
                .flatten()
                .filter(|t| now - **t < FAULT_WINDOW_MS)
                .count() as u32,
            last_activity: s.last_activity,
            snippet: s.snippet.map(|(_, t)| t),
        }
    }
}

/// Agents an event is about: its subject, the job owner in task payloads,
/// agent parties of a message.
fn attributed(e: &Event) -> Vec<&str> {
    let mut out = Vec::with_capacity(2);
    if e.subject_type.as_deref() == Some("agent")
        && let Some(id) = e.subject_id.as_deref()
    {
        out.push(id);
    }
    for key in ["owner_id", "prior_owner", "agent_id"] {
        if let Some(id) = e.payload.get(key).and_then(|v| v.as_str()) {
            out.push(id);
        }
    }
    if e.kind == EventKind::MessageCreated {
        for side in ["from", "to"] {
            let party = &e.payload[side];
            if party["kind"] == "agent"
                && let Some(id) = party["id"].as_str()
            {
                out.push(id);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The agent a fault event counts against (S4.B health).
fn fault_owner(e: &Event) -> Option<&str> {
    let key = match e.kind {
        EventKind::TaskLeasedOut => "prior_owner",
        EventKind::TaskFailed => "owner_id",
        EventKind::ApprovalExpired => "agent_id",
        _ => return None,
    };
    e.payload.get(key).and_then(|v| v.as_str())
}

/// Last few non-blank lines, control characters dropped, lines clipped.
fn snippet(text: &str) -> Option<String> {
    let lines: Vec<String> = text
        .lines()
        .map(|l| {
            l.chars()
                .filter(|c| !c.is_control())
                .take(SNIPPET_LINE_CHARS)
                .collect::<String>()
        })
        .filter(|l| !l.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return None;
    }
    Some(lines[lines.len().saturating_sub(SNIPPET_LINES)..].join("\n"))
}

/// Point radius from activity: `7 + 11 · min(1, ln(1+n) / ln(121))` px.
pub fn radius(activity: u32) -> f64 {
    7.0 + 11.0 * ((1.0 + f64::from(activity)).ln() / 121f64.ln()).min(1.0)
}

/// Inputs to the health score besides metrics.
#[derive(Debug, Clone, Copy)]
pub struct HealthInputs {
    pub state: AgentState,
    /// Fraction of the owned job's `lease_s` still left, if it owns one.
    pub lease_left: Option<f64>,
}

/// Brightness in [0.35, 1] (S4.B).
pub fn health(inp: HealthInputs, m: &AgentMetrics, now: i64) -> f64 {
    if inp.state == AgentState::Dead {
        return 0.35;
    }
    let mut h: f64 = 1.0;
    if inp.state == AgentState::Unknown {
        h *= 0.5;
    }
    if inp.lease_left.is_some_and(|f| f < 0.25) {
        h *= 0.6;
    }
    h *= 0.6f64.powi(m.faults.min(2) as i32);
    let silent = m.last_activity.is_none_or(|t| now - t >= SILENCE_MS);
    if inp.state == AgentState::Working && silent {
        h *= 0.7;
    }
    h.clamp(0.35, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(
        seq: i64,
        ts: i64,
        kind: EventKind,
        subject: Option<&str>,
        payload: serde_json::Value,
    ) -> Event {
        Event {
            seq,
            ts,
            kind,
            subject_type: subject.map(|_| "agent".to_string()),
            subject_id: subject.map(str::to_string),
            payload,
        }
    }

    const T0: i64 = 1_000_000_000_000;

    #[test]
    fn activity_counts_only_the_last_five_minutes() {
        let mut m = Metrics::default();
        // one event per 10 s bucket over 8 minutes
        for i in 0..48 {
            m.apply(&ev(
                i,
                T0 + i * BUCKET_MS,
                EventKind::AgentOutput,
                Some("a1"),
                serde_json::json!({"text": "x"}),
            ));
        }
        let now = T0 + 47 * BUCKET_MS;
        let a = m.agent("a1", "alpha", now);
        assert_eq!(a.activity, 30, "only the 30 newest buckets");
        assert_eq!(a.series, [1; BUCKETS]);

        // time moves on without events: the window drains
        let later = m.agent("a1", "alpha", now + 10 * BUCKET_MS);
        assert_eq!(later.activity, 20);
        assert_eq!(later.series[BUCKETS - 1], 0);
        assert_eq!(m.agent("a1", "alpha", now + ACTIVITY_WINDOW_MS).activity, 0);
    }

    #[test]
    fn messages_attribute_to_agent_parties_by_name_or_id() {
        let mut m = Metrics::default();
        let msg = |seq, from: (&str, &str), to: (&str, &str)| {
            ev(
                seq,
                T0,
                EventKind::MessageCreated,
                None,
                serde_json::json!({
                    "kind": "notice",
                    "from": {"kind": from.0, "id": from.1},
                    "to": {"kind": to.0, "id": to.1},
                }),
            )
        };
        m.apply(&msg(1, ("agent", "a1"), ("agent", "beta")));
        m.apply(&msg(2, ("human", "me"), ("agent", "alpha")));
        let a = m.agent("a1", "alpha", T0);
        assert_eq!(a.activity, 2, "id key + name key merged");
        assert!((a.msgs_per_min - 0.4).abs() < 1e-9);
        assert_eq!(m.agent("b1", "beta", T0).activity, 1);
        assert_eq!(
            m.agent("me", "me", T0).activity,
            0,
            "humans are not tracked"
        );
    }

    #[test]
    fn faults_count_for_fifteen_minutes_and_dim_health() {
        let mut m = Metrics::default();
        m.apply(&ev(
            1,
            T0,
            EventKind::TaskLeasedOut,
            None,
            serde_json::json!({"task_id": "t", "prior_owner": "a1"}),
        ));
        m.apply(&ev(
            2,
            T0 + 1,
            EventKind::TaskFailed,
            None,
            serde_json::json!({"task_id": "t", "owner_id": "a1"}),
        ));
        m.apply(&ev(
            3,
            T0 + 2,
            EventKind::ApprovalExpired,
            None,
            serde_json::json!({"agent_id": "a1"}),
        ));
        let now = T0 + 60_000;
        let a = m.agent("a1", "alpha", now);
        assert_eq!(a.faults, 2, "two most recent are kept");
        let idle = HealthInputs {
            state: AgentState::Idle,
            lease_left: None,
        };
        assert!((health(idle, &a, now) - 0.36).abs() < 1e-9);

        let after = m.agent("a1", "alpha", T0 + 2 + FAULT_WINDOW_MS);
        assert_eq!(after.faults, 0);
        assert_eq!(health(idle, &after, T0 + 2 + FAULT_WINDOW_MS), 1.0);
    }

    #[test]
    fn health_factors_and_clamp() {
        let m = Metrics::default();
        let quiet = m.agent("a1", "a1", T0);
        let h = |state, lease_left| health(HealthInputs { state, lease_left }, &quiet, T0);
        assert_eq!(h(AgentState::Idle, None), 1.0);
        assert_eq!(h(AgentState::Unknown, None), 0.5);
        assert_eq!(h(AgentState::Idle, Some(0.2)), 0.6, "heartbeat overdue");
        assert_eq!(h(AgentState::Idle, Some(0.3)), 1.0);
        assert_eq!(h(AgentState::Working, None), 0.7, "working but silent");
        assert_eq!(h(AgentState::Dead, None), 0.35);
        assert_eq!(h(AgentState::Unknown, Some(0.0)), 0.35, "clamped");
    }

    #[test]
    fn radius_grows_logarithmically_and_caps() {
        assert_eq!(radius(0), 7.0);
        assert!(radius(10) > radius(1));
        assert!((radius(120) - 18.0).abs() < 1e-9);
        assert_eq!(radius(10_000), 18.0);
    }

    #[test]
    fn removal_forgets_the_agent_and_prune_bounds_memory() {
        let mut m = Metrics::default();
        m.apply(&ev(
            1,
            T0,
            EventKind::AgentOutput,
            Some("a1"),
            serde_json::json!({"text": "hello\n\nworld"}),
        ));
        m.apply(&ev(
            2,
            T0,
            EventKind::AgentOutput,
            Some("a2"),
            serde_json::json!({"text": "x"}),
        ));
        assert_eq!(
            m.agent("a1", "a1", T0).snippet.as_deref(),
            Some("hello\nworld")
        );
        m.apply(&ev(
            3,
            T0,
            EventKind::AgentRemoved,
            Some("a1"),
            serde_json::json!({"name": "alpha"}),
        ));
        assert_eq!(m.agent("a1", "alpha", T0).activity, 0);
        assert_eq!(m.tracked(), 1);
        m.prune(T0 + FAULT_WINDOW_MS);
        assert_eq!(m.tracked(), 0);
    }

    #[test]
    fn ribbon_keeps_the_last_ten_non_output_events() {
        let mut m = Metrics::default();
        for i in 0..15 {
            m.apply(&ev(
                i,
                T0,
                EventKind::AgentStateChange,
                Some("a1"),
                serde_json::json!({}),
            ));
            m.apply(&ev(
                100 + i,
                T0,
                EventKind::AgentOutput,
                Some("a1"),
                serde_json::json!({"text": "x"}),
            ));
        }
        let seqs: Vec<i64> = m.ribbon().map(|e| e.seq).collect();
        assert_eq!(seqs, (5..15).collect::<Vec<_>>());
    }
}
