//! `tower task …` (D§10, plan T3.4): the operator's job-queue surface.

use std::collections::HashMap;

use clap::Subcommand;
use serde_json::{json, Value};
use tower_client::Client;

#[derive(Debug, Subcommand)]
pub enum TaskCmd {
    /// Queue + owned views, ordered by priority then age
    List {
        #[arg(long)]
        state: Option<String>,
        /// Require this tag (repeatable; all must match)
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Open jobs owned by this agent
        #[arg(long)]
        mine: Option<String>,
    },
    /// Detail incl. the assignment/lease trail
    Show { id: String },
    /// Queue a job (optionally pre-assigned). No title in a terminal: asks
    /// for every value no flag gave
    Create {
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long)]
        priority: Option<i64>,
        /// Pre-assign to this agent
        #[arg(long)]
        assign: Option<String>,
        /// With --assign: reserve it — delivered when the agent is free
        #[arg(long)]
        when_available: bool,
        /// With --assign: reserve it and hold it until this time
        /// (HH:MM today/tomorrow, "YYYY-MM-DD HH:MM" local, or RFC 3339)
        #[arg(long)]
        at: Option<String>,
        /// Lease window in seconds (default 60)
        #[arg(long)]
        lease_s: Option<i64>,
        /// Lifecycle prompt typed into the harness before the job prompt,
        /// on every delivery (e.g. '/clear')
        #[arg(long)]
        pre_run: Option<String>,
        /// Lifecycle prompt typed after the owner reports completed or
        /// failed (e.g. '/reflect')
        #[arg(long)]
        post_run: Option<String>,
    },
    /// Edit an open job: a queued job takes every field, a delivered one
    /// only --post-run ('' clears a field)
    Edit {
        id: String,
        #[arg(long)]
        title: Option<String>,
        /// The job prompt
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        pre_run: Option<String>,
        #[arg(long)]
        post_run: Option<String>,
        /// Reserved jobs: hold until this time (HH:MM today/tomorrow,
        /// "YYYY-MM-DD HH:MM" local, or RFC 3339)
        #[arg(long)]
        at: Option<String>,
    },
    /// Dispatch a queued job to an agent (one job per agent at a time)
    Assign {
        id: String,
        name: String,
        #[arg(long)]
        lease_s: Option<i64>,
        /// Reserve instead: deliver when the agent is free
        #[arg(long)]
        when_available: bool,
    },
    /// Cancel a job (interrupts its owner)
    Cancel { id: String },
    /// Give a job back to the queue (as $TOWER_AGENT, else as its owner)
    Release {
        id: String,
        #[arg(long = "as", env = "TOWER_AGENT")]
        as_agent: Option<String>,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Work loop: declare you started your assigned job
    Start {
        id: String,
        #[arg(long = "as", env = "TOWER_AGENT")]
        as_agent: String,
    },
    /// Work loop: renew your job lease (every lease_s/3 seconds)
    Heartbeat {
        id: String,
        #[arg(long = "as", env = "TOWER_AGENT")]
        as_agent: String,
    },
    /// Work loop: report progress or finish your job
    Status {
        id: String,
        /// working | input-required | completed | failed
        state: String,
        /// Result summary (JSON, or plain text stored as {"summary": ...})
        #[arg(long)]
        result: Option<String>,
        #[arg(long = "as", env = "TOWER_AGENT")]
        as_agent: String,
    },
}

pub async fn run(c: &Client, cmd: TaskCmd, json_out: bool) -> anyhow::Result<()> {
    match cmd {
        TaskCmd::List { state, tags, mine } => {
            let mut q = Vec::new();
            if let Some(s) = state {
                q.push(format!("state={s}"));
            }
            if !tags.is_empty() {
                q.push(format!("tags={}", tags.join(",")));
            }
            if let Some(m) = mine {
                q.push(format!("mine={m}"));
            }
            let v = c.get(&format!("/v1/tasks?{}", q.join("&"))).await?;
            if json_out {
                return print_json(&v);
            }
            let names = agent_names(c).await?;
            print_list(
                v["tasks"].as_array().map(Vec::as_slice).unwrap_or(&[]),
                &names,
            );
        }
        TaskCmd::Show { id } => {
            let v = c.get(&format!("/v1/tasks/{id}")).await?;
            if json_out {
                return print_json(&v);
            }
            let names = agent_names(c).await?;
            print_show(&v, &names, tower_core::now_ms());
        }
        TaskCmd::Create {
            title,
            description,
            tags,
            priority,
            assign,
            when_available,
            at,
            lease_s,
            pre_run,
            post_run,
        } => {
            let mut t = super::wizard::NewTask {
                title,
                description,
                tags,
                priority,
                assign,
                when_available,
                at,
                lease_s,
                pre_run,
                post_run,
            };
            if t.title.is_none() {
                super::wizard::require_terminal("the job title")?;
                if !super::wizard::task_create(c, &mut t).await? {
                    eprintln!("not created");
                    return Ok(());
                }
            }
            let not_before =
                t.at.as_deref()
                    .map(|s| tower_client::parse_local_time(s).map_err(|e| e.context("--at")))
                    .transpose()?;
            let body = json!({
                "title": t.title, "description": t.description, "tags": t.tags,
                "priority": t.priority, "assign": t.assign, "lease_s": t.lease_s,
                "when_available": t.when_available, "not_before": not_before,
                "pre_run": t.pre_run, "post_run": t.post_run,
            });
            let v = c.post("/v1/tasks", Some(body)).await?;
            if json_out {
                return print_json(&v);
            }
            let names = agent_names(c).await?;
            println!("{}", summary(&v["task"], &names));
        }
        TaskCmd::Edit {
            id,
            title,
            description,
            pre_run,
            post_run,
            at,
        } => {
            let mut body = serde_json::Map::new();
            let mut changed = Vec::new();
            // '' clears an optional field (null on the wire)
            let text = |s: String| if s.is_empty() { Value::Null } else { json!(s) };
            if let Some(t) = title {
                body.insert("title".into(), json!(t));
                changed.push("title");
            }
            for (key, flag, val) in [
                ("description", "description", description),
                ("pre_run", "pre-run", pre_run),
                ("post_run", "post-run", post_run),
            ] {
                if let Some(v) = val {
                    body.insert(key.into(), text(v));
                    changed.push(flag);
                }
            }
            if let Some(a) = at {
                let nb = if a.trim().is_empty() {
                    Value::Null
                } else {
                    json!(tower_client::parse_local_time(&a).map_err(|e| e.context("--at"))?)
                };
                body.insert("not_before".into(), nb);
                changed.push("at");
            }
            if changed.is_empty() {
                anyhow::bail!(
                    "nothing to change: give --title, --description, --pre-run, --post-run, or --at"
                );
            }
            let v = c
                .patch(&format!("/v1/tasks/{id}"), Value::Object(body))
                .await?;
            if json_out {
                return print_json(&v);
            }
            println!(
                "{}  updated ({})",
                v["task"]["id"].as_str().unwrap_or(&id),
                changed.join(", ")
            );
        }
        TaskCmd::Assign {
            id,
            name,
            lease_s,
            when_available,
        } => {
            let v = c
                .post(
                    &format!("/v1/tasks/{id}/assign"),
                    Some(
                        json!({ "to": name, "lease_s": lease_s, "when_available": when_available }),
                    ),
                )
                .await?;
            if json_out {
                return print_json(&v);
            }
            let names = agent_names(c).await?;
            println!("{}", summary(&v["task"], &names));
        }
        TaskCmd::Cancel { id } => {
            let v = c.post(&format!("/v1/tasks/{id}/cancel"), None).await?;
            if json_out {
                return print_json(&v);
            }
            println!("{id}  canceled");
        }
        TaskCmd::Release {
            id,
            as_agent,
            reason,
        } => {
            let as_agent = match as_agent {
                Some(a) => a,
                None => c.get(&format!("/v1/tasks/{id}")).await?["task"]["owner_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("task {id} has no owner to release"))?
                    .to_string(),
            };
            let v = c
                .post(
                    &format!("/v1/tasks/{id}/release"),
                    Some(json!({ "as": as_agent, "reason": reason })),
                )
                .await?;
            if json_out {
                return print_json(&v);
            }
            println!("{id}  released → queued");
        }
        TaskCmd::Start { id, as_agent } => {
            owner_call(c, &id, "start", json!({ "as": as_agent }), json_out).await?
        }
        TaskCmd::Heartbeat { id, as_agent } => {
            owner_call(c, &id, "heartbeat", json!({ "as": as_agent }), json_out).await?
        }
        TaskCmd::Status {
            id,
            state,
            result,
            as_agent,
        } => {
            let result = result.map(|r| {
                serde_json::from_str::<Value>(&r).unwrap_or_else(|_| json!({ "summary": r }))
            });
            let body = json!({ "as": as_agent, "state": state, "result": result });
            owner_call(c, &id, "status", body, json_out).await?
        }
    }
    Ok(())
}

/// An owner work-loop call; prints `<id>  <state> (lease …)`.
async fn owner_call(
    c: &Client,
    id: &str,
    verb: &str,
    body: Value,
    json_out: bool,
) -> anyhow::Result<()> {
    let v = c
        .post(&format!("/v1/tasks/{id}/{verb}"), Some(body))
        .await?;
    if json_out {
        return print_json(&v);
    }
    let t = &v["task"];
    match t["lease_expires_at"].as_i64() {
        Some(exp) => println!(
            "{id}  {} (lease expires in {}s)",
            t["state"].as_str().unwrap_or("?"),
            (exp - tower_core::now_ms()).max(0) / 1000
        ),
        None => println!("{id}  {}", t["state"].as_str().unwrap_or("?")),
    }
    Ok(())
}

pub(super) fn print_json(v: &Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// Agent id → name, for human output.
pub(super) async fn agent_names(c: &Client) -> anyhow::Result<HashMap<String, String>> {
    let v = c.get("/v1/agents").await?;
    Ok(v["agents"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|a| {
            Some((
                a["id"].as_str()?.to_string(),
                a["name"].as_str()?.to_string(),
            ))
        })
        .collect())
}

pub(super) fn name<'a>(names: &'a HashMap<String, String>, id: &'a Value) -> &'a str {
    match id.as_str() {
        Some(i) => names.get(i).map(String::as_str).unwrap_or(i),
        None => "—",
    }
}

/// `t_…  queued` / `t_…  assigned to backend (lease 60s)`.
fn summary(t: &Value, names: &HashMap<String, String>) -> String {
    let id = t["id"].as_str().unwrap_or("?");
    match t["state"].as_str() {
        Some("assigned") => format!(
            "{id}  assigned to {} (lease {}s{})",
            name(names, &t["owner_id"]),
            t["lease_s"].as_i64().unwrap_or(0),
            if t["hook"] == "pre" {
                ", pre-run first"
            } else {
                ""
            }
        ),
        Some("queued") if t["target_agent_id"].is_string() => {
            let who = name(names, &t["target_agent_id"]);
            match t["not_before"].as_i64() {
                Some(nb) if nb > tower_core::now_ms() => {
                    format!("{id}  reserved for {who} (not before {})", clock(nb, true))
                }
                _ => format!("{id}  reserved for {who} (delivered when available)"),
            }
        }
        Some(s) => format!("{id}  {s}"),
        None => id.to_string(),
    }
}

/// Owner, or `→name` for a job reserved for an agent.
fn owner_cell(t: &Value, names: &HashMap<String, String>) -> String {
    match (t["owner_id"].is_string(), t["target_agent_id"].is_string()) {
        (true, _) => name(names, &t["owner_id"]).to_string(),
        (false, true) => format!("→{}", name(names, &t["target_agent_id"])),
        _ => "—".to_string(),
    }
}

fn print_list(tasks: &[Value], names: &HashMap<String, String>) {
    println!(
        "{:<28} {:<15} {:<10} {:>8}  {:<14} {:<9} TITLE",
        "ID", "STATE", "OWNER", "PRIORITY", "TAGS", "ATTEMPTS"
    );
    for t in tasks {
        let tags = t["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        let title = t["title"].as_str().unwrap_or("");
        let title = if title.chars().count() > 40 {
            format!("{}…", title.chars().take(39).collect::<String>())
        } else {
            title.to_string()
        };
        println!(
            "{:<28} {:<15} {:<10} {:>8}  {:<14} {:<9} {title}",
            t["id"].as_str().unwrap_or("?"),
            t["state"].as_str().unwrap_or("?"),
            owner_cell(t, names),
            t["priority"].as_i64().unwrap_or(0),
            if tags.is_empty() { "—".into() } else { tags },
            format!(
                "{}/{}",
                t["attempt_count"].as_i64().unwrap_or(0),
                t["max_attempts"].as_i64().unwrap_or(0)
            ),
        );
    }
}

fn print_show(v: &Value, names: &HashMap<String, String>, now: i64) {
    let t = &v["task"];
    println!(
        "task {}  {}",
        t["id"].as_str().unwrap_or("?"),
        t["title"].as_str().unwrap_or("")
    );
    let state = t["state"].as_str().unwrap_or("?");
    let owner = name(names, &t["owner_id"]);
    let detail = match t["lease_expires_at"].as_i64() {
        Some(exp) if exp >= now => format!(
            " (owner: {owner}, lease expires in {}s)",
            (exp - now) / 1000
        ),
        Some(_) => format!(" (owner: {owner}, lease expired — requeues on next sweep)"),
        None if t["owner_id"].is_string() => format!(" (owner: {owner})"),
        None if t["target_agent_id"].is_string() => format!(
            " (reserved for {}: delivered when available)",
            name(names, &t["target_agent_id"])
        ),
        None => String::new(),
    };
    println!("  state    {state}{detail}");
    println!(
        "  attempts {}/{}",
        t["attempt_count"].as_i64().unwrap_or(0),
        t["max_attempts"].as_i64().unwrap_or(0)
    );
    println!(
        "  created  {}",
        clock(t["created_at"].as_i64().unwrap_or(0), true)
    );
    for (label, key) in [
        ("about", "description"),
        ("pre-run", "pre_run"),
        ("post-run", "post_run"),
    ] {
        if let Some(text) = t[key].as_str() {
            print_text(label, text);
        }
    }
    match t["hook"].as_str() {
        Some("pre") => println!("  hook     pre-run prompt running (lease paused)"),
        Some("post") => println!("  hook     post-run prompt running (owner busy)"),
        _ => {}
    }
    if !t["result"].is_null() {
        println!("  result   {}", t["result"]);
    }
    println!("  trail");
    for e in v["trail"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        println!(
            "    {}  {}",
            clock(e["ts"].as_i64().unwrap_or(0), false),
            trail_line(e, names)
        );
    }
}

fn trail_line(e: &Value, names: &HashMap<String, String>) -> String {
    let p = &e["payload"];
    match e["kind"].as_str().unwrap_or("?") {
        "task.created" => "created".into(),
        "task.assigned" => format!(
            "assigned to {} by {}",
            name(names, &p["owner_id"]),
            p["by"].as_str().unwrap_or("?")
        ),
        "task.reserved" if p["target"].is_null() => "reservation dropped (target removed)".into(),
        "task.reserved" => {
            let mut s = format!("reserved for {}", name(names, &p["target"]));
            if let Some(nb) = p["not_before"].as_i64() {
                s.push_str(&format!(", not before {}", clock(nb, true)));
            }
            s
        }
        "task.status" if p["released_by"].is_string() => {
            let mut s = format!("released by {}", name(names, &p["released_by"]));
            if let Some(r) = p["reason"].as_str() {
                s.push_str(&format!(" — \"{r}\""));
            }
            s
        }
        "task.status" => match p["state"].as_str() {
            Some("working") => format!("working ({})", name(names, &p["owner_id"])),
            Some(s) => format!("status: {s}"),
            None => "status".into(),
        },
        "task.leased_out" => format!(
            "lease expired ({}) → {}, attempt {}/{}",
            name(names, &p["prior_owner"]),
            p["state"].as_str().unwrap_or("?"),
            p["attempt_count"],
            p["max_attempts"]
        ),
        "task.completed" => format!("completed by {}", name(names, &p["owner_id"])),
        "task.failed" => format!("failed: {}", p["result"]),
        "task.updated" => format!("edited: {}", edited_fields(&p["fields"])),
        "task.hook" => hook_line(p),
        other => other.to_string(),
    }
}

/// `["description","pre_run"]` → `description, pre-run`.
pub fn edited_fields(fields: &Value) -> String {
    fields
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(Value::as_str)
        .map(|f| match f {
            "pre_run" => "pre-run",
            "post_run" => "post-run",
            "not_before" => "at",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `task.hook`: `pre-run prompt sent` / `… done` / `… failed: why`.
pub fn hook_line(p: &Value) -> String {
    let phase = match p["phase"].as_str() {
        Some("post") => "post-run",
        _ => "pre-run",
    };
    match (p["status"].as_str(), p["error"].as_str()) {
        (Some("running"), _) => format!("{phase} prompt sent"),
        (Some("done"), _) => format!("{phase} prompt done"),
        (Some(s), Some(e)) => format!("{phase} prompt {s}: {e}"),
        (Some(s), None) => format!("{phase} prompt {s}"),
        (None, _) => format!("{phase} prompt"),
    }
}

/// Local wall-clock rendering of a ms timestamp.
pub(super) fn clock(ms: i64, with_date: bool) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms).single() {
        Some(t) if with_date => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        Some(t) => t.format("%H:%M:%S").to_string(),
        None => ms.to_string(),
    }
}

/// `  label    text`, later lines indented under the text (prompts span
/// lines).
pub(super) fn print_text(label: &str, text: &str) {
    for (i, line) in text.lines().enumerate() {
        let head = if i == 0 { label } else { "" };
        println!("  {head:<8} {line}");
    }
}
