//! Workflow definitions (D§5.6; decision: workflows §3, §9): a TOML file
//! parsed into a validated graph, and the `{{ path }}` template language
//! its prompts and trigger inputs use.
//!
//! A definition is stored as JSON in `workflow_versions.definition`; the
//! same struct round-trips both ways, so a stored version needs no second
//! validation.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tower_core::{EventKind, TowerError};

/// The caps of a workflow job's result (decision §4, §11.4).
pub const SUMMARY_MAX_CHARS: usize = 2_000;
pub const DATA_MAX_BYTES: usize = 16 * 1024;

/// A validated workflow graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Node whose latest result becomes the run's result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, InputSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<Trigger>,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputSpec {
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
}

/// What starts a run besides a manual start (decision §5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// Always allowed; listing it documents intent only.
    Manual,
    Schedule {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cron: Option<String>,
        /// `HH:MM`, sugar for `M H * * *`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        daily: Option<String>,
        /// IANA zone; the server's zone when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timezone: Option<String>,
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        input: Map<String, Value>,
    },
    Event {
        #[serde(rename = "type")]
        event_type: String,
        #[serde(default, rename = "match", skip_serializing_if = "Map::is_empty")]
        matches: Map<String, Value>,
        /// String values are templates over `{{ event.<path> }}`.
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        input: Map<String, Value>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// A job reserved for an agent (the only kind in phase 7; `exec` is a
    /// follow-up, decision §2).
    #[default]
    Job,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Join {
    /// A step per fired edge.
    #[default]
    Any,
    /// One step once every incoming edge fired (or the run went quiet).
    All,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    #[serde(default)]
    pub kind: NodeKind,
    /// Job title; defaults to `<workflow>: <node>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub agent: AgentRef,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outcomes: Vec<String>,
    /// Nodes whose latest summaries the job carries; default: the nodes of
    /// the steps that activated it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<String>>,
    #[serde(default)]
    pub join: Join,
    /// Loop guard; no limit when absent (every loop needs one guard).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_visits: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_s: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<i64>,
}

/// The node's agent: a name, or a mapping from an earlier node's outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AgentRef {
    Name(String),
    Mapped(AgentMap),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMap {
    pub from: String,
    pub map: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// An outcome of `from`, or `completed` (default), or `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
}

impl Edge {
    /// Whether this edge fires for a step of `from` that ended so.
    pub fn fires(&self, completed: bool, outcome: Option<&str>) -> bool {
        match (self.on.as_deref(), completed) {
            (None | Some("completed"), true) => true,
            (Some("failed"), false) => true,
            (Some(want), true) => outcome == Some(want),
            _ => false,
        }
    }
}

fn invalid(msg: impl Into<String>) -> anyhow::Error {
    TowerError::invalid(msg).into()
}

/// `[A-Za-z0-9][A-Za-z0-9_-]*`, at most 64 chars: names, node ids,
/// outcomes, and input keys (they appear in URLs and template paths).
fn ident(s: &str) -> bool {
    s.len() <= 64
        && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Parse and validate a TOML definition.
pub fn parse(source: &str) -> anyhow::Result<Definition> {
    let def: Definition =
        toml::from_str(source).map_err(|e| invalid(format!("workflow definition: {e}")))?;
    def.validate()?;
    Ok(def)
}

impl Definition {
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn outgoing<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a Edge> + 'a {
        self.edges.iter().filter(move |e| e.from == id)
    }

    /// Distinct source nodes of the edges into `id`.
    pub fn incoming_sources(&self, id: &str) -> BTreeSet<&str> {
        self.edges
            .iter()
            .filter(|e| e.to == id)
            .map(|e| e.from.as_str())
            .collect()
    }

    /// Nodes with no incoming edge: they start at run start.
    pub fn start_nodes(&self) -> Vec<&Node> {
        self.nodes
            .iter()
            .filter(|n| !self.edges.iter().any(|e| e.to == n.id))
            .collect()
    }

    /// Agent names the definition names directly or through a mapping.
    pub fn agent_names(&self) -> BTreeSet<&str> {
        let mut out = BTreeSet::new();
        for n in &self.nodes {
            match &n.agent {
                AgentRef::Name(a) => {
                    out.insert(a.as_str());
                }
                AgentRef::Mapped(m) => out.extend(m.map.values().map(String::as_str)),
            }
        }
        out
    }

    fn validate(&self) -> anyhow::Result<()> {
        if !ident(&self.name) {
            return Err(invalid(format!(
                "workflow name {:?}: use letters, digits, '-' and '_' (at most 64)",
                self.name
            )));
        }
        for key in self.inputs.keys() {
            if !ident(key) {
                return Err(invalid(format!(
                    "input {key:?}: use letters, digits, '-' and '_'"
                )));
            }
        }
        if self.nodes.is_empty() {
            return Err(invalid("a workflow needs at least one node"));
        }
        let mut ids = HashSet::new();
        for n in &self.nodes {
            if !ident(&n.id) {
                return Err(invalid(format!(
                    "node id {:?}: use letters, digits, '-' and '_' (at most 64)",
                    n.id
                )));
            }
            if !ids.insert(n.id.as_str()) {
                return Err(invalid(format!("duplicate node id `{}`", n.id)));
            }
        }
        for n in &self.nodes {
            self.validate_node(n)?;
        }
        let mut seen = HashSet::new();
        for e in &self.edges {
            let Some(from) = self.node(&e.from) else {
                return Err(invalid(format!(
                    "edge {} → {}: no node `{}`",
                    e.from, e.to, e.from
                )));
            };
            if self.node(&e.to).is_none() {
                return Err(invalid(format!(
                    "edge {} → {}: no node `{}`",
                    e.from, e.to, e.to
                )));
            }
            if let Some(on) = &e.on {
                if on != "completed" && on != "failed" && !from.outcomes.contains(on) {
                    return Err(invalid(format!(
                        "edge {} → {} on `{on}`: `{}` declares no such outcome \
                         (use one of its outcomes, `completed`, or `failed`)",
                        e.from, e.to, e.from
                    )));
                }
            }
            if !seen.insert((&e.from, &e.to, &e.on)) {
                return Err(invalid(format!("duplicate edge {} → {}", e.from, e.to)));
            }
        }
        if self.start_nodes().is_empty() {
            return Err(invalid(
                "no start node: every node has an incoming edge, so nothing runs first",
            ));
        }
        self.validate_loops()?;
        if let Some(r) = &self.result {
            if self.node(r).is_none() {
                return Err(invalid(format!("result: no node `{r}`")));
            }
        }
        for (i, t) in self.triggers.iter().enumerate() {
            self.validate_trigger(i, t)?;
        }
        Ok(())
    }

    fn validate_node(&self, n: &Node) -> anyhow::Result<()> {
        let at = |msg: String| invalid(format!("node `{}`: {msg}", n.id));
        if n.prompt.trim().is_empty() {
            return Err(at("prompt must not be empty".into()));
        }
        let mut outs = HashSet::new();
        for o in &n.outcomes {
            if !ident(o) {
                return Err(at(format!(
                    "outcome {o:?}: use letters, digits, '-' and '_'"
                )));
            }
            if o == "completed" || o == "failed" {
                return Err(at(format!(
                    "outcome `{o}` is reserved for edges; pick another word"
                )));
            }
            if !outs.insert(o) {
                return Err(at(format!("duplicate outcome `{o}`")));
            }
        }
        match &n.agent {
            AgentRef::Name(a) if a.trim().is_empty() => {
                return Err(at("agent must not be empty".into()))
            }
            AgentRef::Name(_) => {}
            AgentRef::Mapped(m) => {
                let Some(from) = self.node(&m.from).filter(|f| f.id != n.id) else {
                    return Err(at(format!(
                        "agent map: `from = \"{}\"` must name another node",
                        m.from
                    )));
                };
                if m.map.is_empty() {
                    return Err(at("agent map is empty".into()));
                }
                for (outcome, agent) in &m.map {
                    if !from.outcomes.contains(outcome) {
                        return Err(at(format!(
                            "agent map key `{outcome}` is not an outcome of `{}`",
                            from.id
                        )));
                    }
                    if agent.trim().is_empty() {
                        return Err(at(format!("agent map `{outcome}` names no agent")));
                    }
                }
            }
        }
        for c in n.context.iter().flatten() {
            if self.node(c).is_none() {
                return Err(at(format!("context: no node `{c}`")));
            }
        }
        if n.max_visits == Some(0) {
            return Err(at("max_visits must be at least 1".into()));
        }
        if n.lease_s.is_some_and(|s| s <= 0) || n.max_attempts.is_some_and(|m| m < 1) {
            return Err(at("lease_s must be > 0 and max_attempts >= 1".into()));
        }
        let scope = Scope {
            inputs: &self.inputs,
            nodes: Some(self),
            event: false,
        };
        check_template(&n.prompt, &scope).map_err(|e| at(format!("prompt: {e}")))?;
        if let Some(t) = &n.title {
            check_template(t, &scope).map_err(|e| at(format!("title: {e}")))?;
        }
        Ok(())
    }

    /// Every loop needs a guard: a node on it with `max_visits`.
    fn validate_loops(&self) -> anyhow::Result<()> {
        let n = self.nodes.len();
        let index = |id: &str| self.nodes.iter().position(|x| x.id == id);
        let mut adj = vec![Vec::new(); n];
        for e in &self.edges {
            if let (Some(a), Some(b)) = (index(&e.from), index(&e.to)) {
                adj[a].push(b);
            }
        }
        // reach[i]: nodes reachable from i in one or more steps
        let reach: Vec<HashSet<usize>> = (0..n)
            .map(|i| {
                let mut seen = HashSet::new();
                let mut stack = adj[i].clone();
                while let Some(j) = stack.pop() {
                    if seen.insert(j) {
                        stack.extend(adj[j].iter().copied());
                    }
                }
                seen
            })
            .collect();
        let mut done = HashSet::new();
        for i in 0..n {
            if done.contains(&i) || !reach[i].contains(&i) {
                continue;
            }
            let mut cycle: Vec<usize> = (0..n)
                .filter(|&j| j == i || (reach[i].contains(&j) && reach[j].contains(&i)))
                .collect();
            cycle.sort_unstable();
            done.extend(cycle.iter().copied());
            if !cycle.iter().any(|&j| self.nodes[j].max_visits.is_some()) {
                let names: Vec<String> = cycle
                    .iter()
                    .map(|&j| format!("`{}`", self.nodes[j].id))
                    .collect();
                return Err(invalid(format!(
                    "loop through {} has no guard: set max_visits on one of its nodes",
                    names.join(", ")
                )));
            }
        }
        Ok(())
    }

    fn validate_trigger(&self, i: usize, t: &Trigger) -> anyhow::Result<()> {
        let at = |msg: String| invalid(format!("trigger {}: {msg}", i + 1));
        let input = match t {
            Trigger::Manual => return Ok(()),
            Trigger::Schedule {
                cron,
                daily,
                timezone,
                input,
            } => {
                let expr = match (cron, daily) {
                    (Some(c), None) => c.trim().to_string(),
                    (None, Some(d)) => crate::schedules::daily_cron(d)?,
                    _ => return Err(at("give exactly one of `cron` or `daily`".into())),
                };
                crate::schedules::parse_cron(&expr)?;
                if let Some(tz) = timezone {
                    crate::schedules::parse_tz(tz)?;
                }
                for (k, v) in input {
                    if v.as_str().is_some_and(|s| s.contains("{{")) {
                        return Err(at(format!(
                            "input `{k}`: schedule inputs are literal (no templates)"
                        )));
                    }
                }
                input
            }
            Trigger::Event {
                event_type,
                matches,
                input,
            } => {
                serde_json::from_value::<EventKind>(Value::String(event_type.clone()))
                    .map_err(|_| at(format!("unknown event type `{event_type}`")))?;
                if matches.keys().any(|k| k.is_empty()) {
                    return Err(at("match keys must not be empty".into()));
                }
                let scope = Scope {
                    inputs: &self.inputs,
                    nodes: None,
                    event: true,
                };
                for (k, v) in input {
                    if let Some(s) = v.as_str() {
                        check_template(s, &scope).map_err(|e| at(format!("input `{k}`: {e}")))?;
                    }
                }
                input
            }
        };
        for k in input.keys() {
            if !self.inputs.contains_key(k) {
                return Err(at(format!("input `{k}` is not declared in [inputs]")));
            }
        }
        for (k, spec) in &self.inputs {
            if spec.required && spec.default.is_none() && !input.contains_key(k) {
                return Err(at(format!(
                    "required input `{k}` has no value (give it in the trigger's input or a default)"
                )));
            }
        }
        Ok(())
    }
}

// ---- templates -------------------------------------------------------------

/// What a template may refer to: prompts and titles see `input.*`,
/// `run.id`, and `nodes.*`; event-trigger inputs see `event.*` only.
struct Scope<'a> {
    inputs: &'a BTreeMap<String, InputSpec>,
    /// `nodes.*` and `run.id` (prompts and titles).
    nodes: Option<&'a Definition>,
    /// `event.*` instead of everything else (event-trigger inputs).
    event: bool,
}

/// The `{{ path }}` placeholders of `s`: (start, end, path) byte ranges.
fn placeholders(s: &str) -> Result<Vec<(usize, usize, &str)>, String> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(open) = s[from..].find("{{") {
        let open = from + open;
        let Some(close) = s[open + 2..].find("}}") else {
            return Err("unclosed `{{`".into());
        };
        let close = open + 2 + close;
        let path = s[open + 2..close].trim();
        let valid = !path.is_empty()
            && path.split('.').all(|seg| {
                !seg.is_empty()
                    && seg
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            });
        if !valid {
            return Err(format!("`{{{{ {path} }}}}` is not a dotted path"));
        }
        out.push((open, close + 2, path));
        from = close + 2;
    }
    Ok(out)
}

fn check_template(s: &str, scope: &Scope) -> Result<(), String> {
    for (_, _, path) in placeholders(s)? {
        let segs: Vec<&str> = path.split('.').collect();
        let ok = match segs.as_slice() {
            ["input", key] => !scope.event && scope.inputs.contains_key(*key),
            ["run", "id"] => scope.nodes.is_some(),
            ["nodes", node, rest @ ..] => {
                scope.nodes.is_some_and(|d| d.node(node).is_some())
                    && matches!(rest, ["outcome"] | ["summary"] | ["data", ..])
            }
            ["event", _, ..] => scope.event,
            _ => false,
        };
        if !ok {
            let allowed = if scope.event {
                "event.<path>"
            } else {
                "input.<declared input>, run.id, nodes.<id>.outcome|summary|data.<path>"
            };
            return Err(format!("unknown name `{path}` (use {allowed})"));
        }
    }
    Ok(())
}

/// Render `s` against `ctx` (an object with `input`, `run`, `nodes`,
/// `event`). Strings render as they are, other values as JSON, a missing
/// value as empty text.
pub fn render(s: &str, ctx: &Value) -> String {
    let Ok(found) = placeholders(s) else {
        return s.to_string(); // validated at apply time; never here
    };
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    for (start, end, path) in found {
        out.push_str(&s[at..start]);
        let mut v = ctx;
        for seg in path.split('.') {
            v = v.get(seg).unwrap_or(&Value::Null);
        }
        match v {
            Value::Null => {}
            Value::String(text) => out.push_str(text),
            other => out.push_str(&other.to_string()),
        }
        at = end;
    }
    out.push_str(&s[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISSUE: &str = r#"
name = "issue-to-done"
result = "review"

[inputs]
issue = { required = true, description = "Issue id" }

[[nodes]]
id = "triage"
agent = "triager"
outcomes = ["build", "design"]
prompt = "Read issue {{ input.issue }}."

[[nodes]]
id = "work"
agent = { from = "triage", map = { build = "builder", design = "designer" } }
context = ["triage", "review"]
max_visits = 3
prompt = "Do the work for {{ input.issue }} ({{ nodes.triage.outcome }})."

[[nodes]]
id = "review"
agent = "reviewer"
outcomes = ["approved", "changes"]
prompt = "Review {{ input.issue }}."

[[edges]]
from = "triage"
to = "work"

[[edges]]
from = "work"
to = "review"

[[edges]]
from = "review"
to = "work"
on = "changes"
"#;

    const DIGEST: &str = r#"
name = "daily-digest"

[[triggers]]
kind = "schedule"
cron = "0 8 * * MON-FRI"
timezone = "America/Vancouver"

[[nodes]]
id = "slack"
agent = "slack-scout"
prompt = "Collect."

[[nodes]]
id = "linear"
agent = "linear-scout"
prompt = "Collect."

[[nodes]]
id = "aggregate"
agent = "librarian"
join = "all"
context = ["slack", "linear"]
prompt = "Merge."

[[edges]]
from = "slack"
to = "aggregate"

[[edges]]
from = "linear"
to = "aggregate"
"#;

    fn err(src: &str) -> String {
        parse(src).unwrap_err().to_string()
    }

    /// A one-node workflow with `extra` appended (more nodes, edges, …).
    fn with(extra: &str) -> String {
        format!("name = \"w\"\n[[nodes]]\nid = \"a\"\nagent = \"x\"\nprompt = \"p\"\n{extra}")
    }

    #[test]
    fn both_decision_examples_parse_and_round_trip_through_json() {
        for src in [ISSUE, DIGEST] {
            let def = parse(src).unwrap();
            let json = serde_json::to_string(&def).unwrap();
            assert_eq!(serde_json::from_str::<Definition>(&json).unwrap(), def);
        }
        let def = parse(ISSUE).unwrap();
        assert_eq!(
            def.start_nodes()
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            ["triage"]
        );
        assert_eq!(
            def.agent_names().into_iter().collect::<Vec<_>>(),
            ["builder", "designer", "reviewer", "triager"]
        );
        let digest = parse(DIGEST).unwrap();
        assert_eq!(digest.node("aggregate").unwrap().join, Join::All);
        assert_eq!(digest.incoming_sources("aggregate").len(), 2);
    }

    #[test]
    fn edges_fire_on_their_outcome_completed_or_failed() {
        let e = |on: Option<&str>| Edge {
            from: "a".into(),
            to: "b".into(),
            on: on.map(str::to_string),
        };
        assert!(e(None).fires(true, Some("x")));
        assert!(e(Some("completed")).fires(true, None));
        assert!(!e(None).fires(false, None));
        assert!(e(Some("failed")).fires(false, None));
        assert!(e(Some("x")).fires(true, Some("x")));
        assert!(!e(Some("x")).fires(true, Some("y")));
        assert!(!e(Some("x")).fires(false, Some("x")));
    }

    #[test]
    fn unknown_keys_and_node_kinds_are_refused() {
        assert!(err(&with("colour = \"red\"")).contains("unknown field"));
        assert!(err(&with(
            "[[nodes]]\nid = \"b\"\nkind = \"exec\"\nagent = \"x\"\nprompt = \"p\""
        ))
        .contains("exec"));
    }

    #[test]
    fn graph_rules_name_the_node_or_edge() {
        assert!(err(&with(
            "[[nodes]]\nid = \"a\"\nagent = \"y\"\nprompt = \"p\""
        ))
        .contains("duplicate node id `a`"));
        assert!(err(&with("[[edges]]\nfrom = \"a\"\nto = \"zz\"")).contains("no node `zz`"));
        assert!(
            err(&with("[[edges]]\nfrom = \"a\"\nto = \"a\"\non = \"maybe\""))
                .contains("declares no such outcome")
        );
        assert!(err(&with(
            "[[nodes]]\nid = \"b\"\nagent = \"y\"\nprompt = \"p\"\noutcomes = [\"failed\"]"
        ))
        .contains("reserved"));
        assert!(err(&with(
            "[[nodes]]\nid = \"b\"\nagent = \"y\"\nprompt = \" \""
        ))
        .contains("node `b`: prompt must not be empty"));
        let bad_result = "name = \"w\"\nresult = \"nope\"\n[[nodes]]\nid = \"a\"\nagent = \"x\"\nprompt = \"p\"\n";
        assert!(err(bad_result).contains("result: no node `nope`"));
    }

    #[test]
    fn a_loop_needs_a_guard_and_a_graph_needs_a_start() {
        let unguarded = ISSUE.replace("max_visits = 3\n", "");
        assert!(err(&unguarded).contains("loop through `work`, `review` has no guard"));
        let self_loop = with("[[edges]]\nfrom = \"a\"\nto = \"a\"");
        assert!(err(&self_loop).contains("no start node"));
        let guarded_self_loop = with(
            "[[nodes]]\nid = \"b\"\nagent = \"y\"\nprompt = \"p\"\nmax_visits = 2\n\
             [[edges]]\nfrom = \"a\"\nto = \"b\"\n[[edges]]\nfrom = \"b\"\nto = \"b\"",
        );
        parse(&guarded_self_loop).unwrap();
    }

    #[test]
    fn agent_maps_must_follow_another_nodes_outcomes() {
        let bad_key = ISSUE.replace("design = \"designer\"", "ops = \"designer\"");
        assert!(err(&bad_key).contains("agent map key `ops` is not an outcome of `triage`"));
        let bad_from = ISSUE.replace("from = \"triage\"", "from = \"nobody\"");
        assert!(err(&bad_from).contains("must name another node"));
    }

    #[test]
    fn templates_are_checked_at_parse_time() {
        assert!(
            err(&ISSUE.replace("{{ input.issue }}.", "{{ input.ticket }}."))
                .contains("unknown name `input.ticket`")
        );
        assert!(
            err(&ISSUE.replace("{{ nodes.triage.outcome }}", "{{ nodes.nope.outcome }}"))
                .contains("nodes.nope.outcome")
        );
        assert!(
            err(&ISSUE.replace("{{ nodes.triage.outcome }}", "{{ nodes.triage.agent }}"))
                .contains("nodes.triage.agent")
        );
        assert!(err(&ISSUE.replace("{{ input.issue }}.", "{{ input.issue .")).contains("unclosed"));
        parse(&ISSUE.replace(
            "{{ nodes.triage.outcome }}",
            "{{ nodes.triage.data.size.w }} {{ run.id }}",
        ))
        .unwrap();
    }

    #[test]
    fn triggers_check_cadence_event_type_and_inputs() {
        let digest_tz = DIGEST.replace("America/Vancouver", "Mars/Olympus");
        assert!(err(&digest_tz).contains("unknown timezone"));
        let both = DIGEST.replace(
            "cron = \"0 8 * * MON-FRI\"",
            "cron = \"0 8 * * *\"\ndaily = \"08:00\"",
        );
        assert!(err(&both).contains("exactly one of `cron` or `daily`"));
        let required = format!(
            "{ISSUE}\n[[triggers]]\nkind = \"event\"\ntype = \"agent.state\"\nmatch = {{ to = \"blocked\" }}\n"
        );
        assert!(err(&required).contains("required input `issue` has no value"));
        let ok = format!(
            "{ISSUE}\n[[triggers]]\nkind = \"event\"\ntype = \"agent.state\"\nmatch = {{ to = \"blocked\" }}\ninput = {{ issue = \"{{{{ event.subject_id }}}}\" }}\n"
        );
        parse(&ok).unwrap();
        let bad_type = ok.replace("agent.state", "agent.sneezed");
        assert!(err(&bad_type).contains("unknown event type `agent.sneezed`"));
        let bad_path = ok.replace("event.subject_id", "input.issue");
        assert!(err(&bad_path).contains("unknown name `input.issue`"));
        let undeclared = ok.replace("input = { issue", "input = { ticket");
        assert!(err(&undeclared).contains("input `ticket` is not declared"));
    }

    #[test]
    fn render_fills_paths_and_leaves_missing_ones_empty() {
        let ctx = serde_json::json!({
            "input": {"issue": "ENG-1", "n": 3},
            "nodes": {"triage": {"outcome": "build", "data": {"size": {"w": 2}}}},
        });
        assert_eq!(
            render("{{input.issue}}/{{ input.n }}/{{ nodes.triage.outcome }}/{{ nodes.triage.data.size }}/{{ nodes.work.summary }}.", &ctx),
            "ENG-1/3/build/{\"w\":2}/."
        );
        assert_eq!(render("no placeholders", &ctx), "no placeholders");
    }
}
