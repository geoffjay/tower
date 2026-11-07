//! `tower workflow …` and `tower run …` (D§10, phase 7 T5.1): workflow
//! definitions and their runs.

use std::collections::HashMap;
use std::path::PathBuf;

use clap::Subcommand;
use serde_json::{json, Value};
use tower_client::Client;

use super::task::{agent_names, clock, name, print_json, print_text};

#[derive(Debug, Subcommand)]
pub enum WorkflowCmd {
    /// Validate a TOML definition and store it as a new version when it
    /// changed (schedule triggers are synced)
    Apply { file: PathBuf },
    /// Workflows with version, triggers, and running runs
    List,
    /// A workflow's nodes, edges, triggers, and recent runs
    Show { name: String },
    /// Remove a workflow and its schedules (refused while a run is running)
    Rm { name: String },
    /// Allow runs again; resume its schedules
    Enable { name: String },
    /// Refuse manual starts, pause its schedules, stop its event triggers
    Disable { name: String },
    /// Start a run of the current version. No name in a terminal: pick the
    /// workflow and answer its inputs
    Run {
        name: Option<String>,
        /// Input value, repeatable: --input issue=ENG-123
        #[arg(long = "input", value_name = "KEY=VALUE")]
        inputs: Vec<String>,
        /// Follow the run until it ends (exit 1 unless it completes)
        #[arg(long)]
        wait: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum RunCmd {
    /// Runs, newest first
    List {
        #[arg(long)]
        workflow: Option<String>,
        /// running | completed | failed | canceled
        #[arg(long)]
        state: Option<String>,
    },
    /// A run's steps with agent, outcome, and summary
    Show { id: String },
    /// Cancel a running run and its open jobs
    Cancel { id: String },
}

pub async fn run_workflow(c: &Client, cmd: WorkflowCmd, json_out: bool) -> anyhow::Result<()> {
    match cmd {
        WorkflowCmd::Apply { file } => {
            let source = std::fs::read_to_string(&file)
                .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
            let v = c
                .post("/v1/workflows", Some(json!({ "source": source })))
                .await?;
            if json_out {
                return print_json(&v);
            }
            let w = &v["workflow"];
            let version = v["version"].as_i64().unwrap_or(0);
            let what = match (v["changed"].as_bool(), version) {
                (Some(true), 1) => "created",
                (Some(true), _) => "new version",
                _ => "unchanged",
            };
            println!("{}  v{version} ({what})", w["name"].as_str().unwrap_or("?"));
        }
        WorkflowCmd::List => {
            let v = c.get("/v1/workflows").await?;
            if json_out {
                return print_json(&v);
            }
            print_list(v["workflows"].as_array().map(Vec::as_slice).unwrap_or(&[]));
        }
        WorkflowCmd::Show { name } => {
            let v = c.get(&format!("/v1/workflows/{name}")).await?;
            if json_out {
                return print_json(&v);
            }
            print_show(&v);
        }
        WorkflowCmd::Rm { name } => {
            let v = c.delete(&format!("/v1/workflows/{name}")).await?;
            if json_out {
                return print_json(&v);
            }
            println!("{name}  removed");
        }
        WorkflowCmd::Enable { name } => toggle(c, &name, "enable", json_out).await?,
        WorkflowCmd::Disable { name } => toggle(c, &name, "disable", json_out).await?,
        WorkflowCmd::Run {
            mut name,
            mut inputs,
            mut wait,
        } => {
            if name.is_none() {
                super::wizard::require_terminal("the workflow name")?;
                if !super::wizard::workflow_run(c, &mut name, &mut inputs, &mut wait).await? {
                    eprintln!("not started");
                    return Ok(());
                }
            }
            let name = name.expect("asked above");
            let mut input = serde_json::Map::new();
            for kv in inputs {
                let (k, v) = kv
                    .split_once('=')
                    .ok_or_else(|| anyhow::anyhow!("--input {kv:?}: use KEY=VALUE"))?;
                input.insert(k.trim().to_string(), json!(v));
            }
            let v = c
                .post(
                    &format!("/v1/workflows/{name}/runs"),
                    Some(json!({ "input": input })),
                )
                .await?;
            let run = &v["run"];
            let id = run["id"].as_str().unwrap_or("?").to_string();
            if !wait {
                if json_out {
                    return print_json(&v);
                }
                println!(
                    "run {id}  started ({} v{})",
                    run["workflow_name"].as_str().unwrap_or("?"),
                    run["version"]
                );
                return Ok(());
            }
            if !json_out {
                println!(
                    "run {id}  started ({} v{})",
                    run["workflow_name"].as_str().unwrap_or("?"),
                    run["version"]
                );
            }
            follow(c, &id, json_out).await?;
        }
    }
    Ok(())
}

async fn toggle(c: &Client, name: &str, verb: &str, json_out: bool) -> anyhow::Result<()> {
    let v = c
        .post(&format!("/v1/workflows/{name}/{verb}"), None)
        .await?;
    if json_out {
        return print_json(&v);
    }
    println!("{name}  {verb}d");
    Ok(())
}

/// Poll the run every 2 s, printing each step change, until it ends.
async fn follow(c: &Client, id: &str, json_out: bool) -> anyhow::Result<()> {
    let mut seen: HashMap<String, String> = HashMap::new();
    let names = agent_names(c).await?;
    loop {
        let v = c.get(&format!("/v1/runs/{id}")).await?;
        if !json_out {
            for s in v["steps"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                let sid = s["id"].as_str().unwrap_or_default().to_string();
                let state = s["state"].as_str().unwrap_or("?").to_string();
                if seen.get(&sid) != Some(&state) {
                    println!("  {}", step_line(s, &names));
                    seen.insert(sid, state);
                }
            }
        }
        let run = &v["run"];
        match run["state"].as_str() {
            Some("running") => tokio::time::sleep(std::time::Duration::from_secs(2)).await,
            Some(end) => {
                if json_out {
                    print_json(&v)?;
                } else {
                    println!("run {id}  {end}{}", run_tail(run));
                }
                if end == "completed" {
                    return Ok(());
                }
                anyhow::bail!("run {id} {end}");
            }
            None => anyhow::bail!("run {id}: unexpected response"),
        }
    }
}

/// `: <error>` or ` → <node> <outcome>: <summary>`.
fn run_tail(run: &Value) -> String {
    if let Some(e) = run["error"].as_str() {
        return format!(": {e}");
    }
    let r = &run["result"];
    match r["node"].as_str() {
        Some(node) => {
            let mut s = format!(" → {node}");
            if let Some(o) = r["outcome"].as_str() {
                s.push_str(&format!(" {o}"));
            }
            if let Some(sum) = r["summary"].as_str() {
                s.push_str(&format!(": {}", first_line(sum)));
            }
            s
        }
        None => String::new(),
    }
}

fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    if line.chars().count() > 80 {
        format!("{}…", line.chars().take(79).collect::<String>())
    } else {
        line.to_string()
    }
}

/// `triage #1  completed  triager  → design`.
fn step_line(s: &Value, names: &HashMap<String, String>) -> String {
    let mut line = format!(
        "{} #{}  {}",
        s["node_id"].as_str().unwrap_or("?"),
        s["visit"],
        s["state"].as_str().unwrap_or("?")
    );
    if s["agent_id"].is_string() {
        line.push_str(&format!("  {}", name(names, &s["agent_id"])));
    }
    if let Some(o) = s["outcome"].as_str() {
        line.push_str(&format!("  → {o}"));
    }
    if let Some(e) = s["error"].as_str() {
        line.push_str(&format!("  ({e})"));
    }
    line
}

fn trigger_summary(t: &Value) -> String {
    match t["kind"].as_str() {
        Some("schedule") => format!(
            "schedule {}",
            t["cron"]
                .as_str()
                .map(str::to_string)
                .or_else(|| t["daily"].as_str().map(|d| format!("daily {d}")))
                .unwrap_or_default()
        ),
        Some("event") => format!("event {}", t["type"].as_str().unwrap_or("?")),
        Some(other) => other.to_string(),
        None => "?".into(),
    }
}

fn print_list(workflows: &[Value]) {
    println!(
        "{:<24} {:<8} {:<9} {:<6} {:<8} {:<30} DESCRIPTION",
        "NAME", "VERSION", "STATE", "NODES", "RUNNING", "TRIGGERS"
    );
    for w in workflows {
        let triggers: Vec<String> = w["triggers"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .map(trigger_summary)
            .collect();
        println!(
            "{:<24} {:<8} {:<9} {:<6} {:<8} {:<30} {}",
            w["name"].as_str().unwrap_or("?"),
            format!("v{}", w["current_version"]),
            if w["enabled"] == true {
                "enabled"
            } else {
                "disabled"
            },
            w["nodes"].to_string(),
            w["open_runs"].to_string(),
            if triggers.is_empty() {
                "manual".into()
            } else {
                triggers.join(", ")
            },
            w["description"].as_str().unwrap_or(""),
        );
    }
}

fn agent_cell(agent: &Value) -> String {
    match agent {
        Value::String(a) => a.clone(),
        Value::Object(m) => {
            let map = m["map"]
                .as_object()
                .map(|map| {
                    map.iter()
                        .map(|(o, a)| format!("{o}→{}", a.as_str().unwrap_or("?")))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            format!("{}: {map}", m["from"].as_str().unwrap_or("?"))
        }
        _ => "?".into(),
    }
}

fn print_show(v: &Value) {
    let w = &v["workflow"];
    let d = &v["definition"];
    println!(
        "workflow {}  v{}  {}",
        w["name"].as_str().unwrap_or("?"),
        w["current_version"],
        if w["enabled"] == true {
            "enabled"
        } else {
            "disabled"
        }
    );
    if let Some(text) = w["description"].as_str() {
        print_text("about", text);
    }
    if let Some(inputs) = d["inputs"].as_object() {
        for (k, spec) in inputs {
            let req = if spec["required"] == true {
                " (required)"
            } else {
                ""
            };
            println!("  input    {k}{req}");
        }
    }
    println!("  nodes");
    for n in d["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let mut line = format!(
            "    {:<16} → {}",
            n["id"].as_str().unwrap_or("?"),
            agent_cell(&n["agent"])
        );
        if let Some(o) = n["outcomes"].as_array() {
            let o: Vec<&str> = o.iter().filter_map(Value::as_str).collect();
            line.push_str(&format!("  outcomes: {}", o.join(", ")));
        }
        if n["join"] == "all" {
            line.push_str("  join: all");
        }
        if let Some(m) = n["max_visits"].as_i64() {
            line.push_str(&format!("  max visits: {m}"));
        }
        println!("{line}");
    }
    let edges = d["edges"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if !edges.is_empty() {
        println!("  edges");
        for e in edges {
            let on = e["on"]
                .as_str()
                .map(|o| format!("  on {o}"))
                .unwrap_or_default();
            println!(
                "    {} → {}{on}",
                e["from"].as_str().unwrap_or("?"),
                e["to"].as_str().unwrap_or("?")
            );
        }
    }
    let triggers = d["triggers"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if !triggers.is_empty() {
        println!("  triggers");
        for t in triggers {
            println!("    {}", trigger_summary(t));
        }
    }
    for s in v["schedules"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let next = match (s["enabled"].as_bool(), s["next_run_at"].as_i64()) {
            (Some(true), Some(n)) => clock(n, true),
            (Some(false), _) => "paused".into(),
            _ => "—".into(),
        };
        println!(
            "  schedule {}  {} {}  next: {next}",
            s["id"].as_str().unwrap_or("?"),
            s["cron"].as_str().unwrap_or(""),
            s["timezone"].as_str().unwrap_or("")
        );
    }
    println!("  runs (newest first)");
    for r in v["runs"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        println!(
            "    {}  {:<10} {}  {}",
            clock(r["created_at"].as_i64().unwrap_or(0), true),
            r["state"].as_str().unwrap_or("?"),
            r["id"].as_str().unwrap_or("?"),
            r["trigger"]["kind"].as_str().unwrap_or("?"),
        );
    }
}

pub async fn run_runs(c: &Client, cmd: RunCmd, json_out: bool) -> anyhow::Result<()> {
    match cmd {
        RunCmd::List { workflow, state } => {
            let mut q = Vec::new();
            if let Some(w) = workflow {
                q.push(format!("workflow={w}"));
            }
            if let Some(s) = state {
                q.push(format!("state={s}"));
            }
            let v = c.get(&format!("/v1/runs?{}", q.join("&"))).await?;
            if json_out {
                return print_json(&v);
            }
            println!(
                "{:<28} {:<20} {:<10} {:<9} {:<20} ENDED",
                "ID", "WORKFLOW", "STATE", "TRIGGER", "STARTED"
            );
            for r in v["runs"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                println!(
                    "{:<28} {:<20} {:<10} {:<9} {:<20} {}",
                    r["id"].as_str().unwrap_or("?"),
                    format!(
                        "{} v{}",
                        r["workflow_name"].as_str().unwrap_or("?"),
                        r["version"]
                    ),
                    r["state"].as_str().unwrap_or("?"),
                    r["trigger"]["kind"].as_str().unwrap_or("?"),
                    clock(r["created_at"].as_i64().unwrap_or(0), true),
                    r["ended_at"]
                        .as_i64()
                        .map(|t| clock(t, true))
                        .unwrap_or_else(|| "—".into()),
                );
            }
        }
        RunCmd::Show { id } => {
            let v = c.get(&format!("/v1/runs/{id}")).await?;
            if json_out {
                return print_json(&v);
            }
            let names = agent_names(c).await?;
            print_run(&v, &names);
        }
        RunCmd::Cancel { id } => {
            let v = c.post(&format!("/v1/runs/{id}/cancel"), None).await?;
            if json_out {
                return print_json(&v);
            }
            println!("run {id}  canceled");
        }
    }
    Ok(())
}

fn print_run(v: &Value, names: &HashMap<String, String>) {
    let r = &v["run"];
    println!(
        "run {}  {} v{}  {}",
        r["id"].as_str().unwrap_or("?"),
        r["workflow_name"].as_str().unwrap_or("?"),
        r["version"],
        r["state"].as_str().unwrap_or("?")
    );
    let t = &r["trigger"];
    let by = t["by"]
        .as_str()
        .or_else(|| t["schedule_id"].as_str())
        .or_else(|| t["event_type"].as_str())
        .map(|b| format!(" ({b})"))
        .unwrap_or_default();
    println!("  trigger  {}{by}", t["kind"].as_str().unwrap_or("?"));
    if let Some(input) = r["input"].as_object().filter(|m| !m.is_empty()) {
        let pairs: Vec<String> = input
            .iter()
            .map(|(k, v)| {
                format!(
                    "{k}={}",
                    v.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| v.to_string())
                )
            })
            .collect();
        println!("  input    {}", pairs.join(" "));
    }
    println!(
        "  started  {}",
        clock(r["created_at"].as_i64().unwrap_or(0), true)
    );
    if let Some(e) = r["ended_at"].as_i64() {
        println!("  ended    {}", clock(e, true));
    }
    let tail = run_tail(r);
    if !tail.is_empty() {
        println!("  result  {tail}");
    }
    println!("  steps");
    for s in v["steps"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let job = s["task_id"]
            .as_str()
            .filter(|_| s["state"] != "pending")
            .map(|t| format!("  job {t}"))
            .unwrap_or_default();
        println!("    {}{job}", step_line(s, names));
        if let Some(sum) = s["result"]["summary"].as_str() {
            for line in sum.lines() {
                println!("        {line}");
            }
        }
    }
}
