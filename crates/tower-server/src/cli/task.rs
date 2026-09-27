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
    /// Queue a job (optionally pre-assigned)
    Create {
        title: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long)]
        priority: Option<i64>,
        /// Pre-assign to this agent
        #[arg(long)]
        assign: Option<String>,
        /// Lease window in seconds (default 60)
        #[arg(long)]
        lease_s: Option<i64>,
    },
    /// Dispatch a queued job to an agent
    Assign {
        id: String,
        name: String,
        #[arg(long)]
        lease_s: Option<i64>,
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
            lease_s,
        } => {
            let body = json!({
                "title": title, "description": description, "tags": tags,
                "priority": priority, "assign": assign, "lease_s": lease_s,
            });
            let v = c.post("/v1/tasks", Some(body)).await?;
            if json_out {
                return print_json(&v);
            }
            let names = agent_names(c).await?;
            println!("{}", summary(&v["task"], &names));
        }
        TaskCmd::Assign { id, name, lease_s } => {
            let v = c
                .post(
                    &format!("/v1/tasks/{id}/assign"),
                    Some(json!({ "to": name, "lease_s": lease_s })),
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

fn print_json(v: &Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// Agent id → name, for human output.
async fn agent_names(c: &Client) -> anyhow::Result<HashMap<String, String>> {
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

fn name<'a>(names: &'a HashMap<String, String>, id: &'a Value) -> &'a str {
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
            "{id}  assigned to {} (lease {}s)",
            name(names, &t["owner_id"]),
            t["lease_s"].as_i64().unwrap_or(0)
        ),
        Some(s) => format!("{id}  {s}"),
        None => id.to_string(),
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
            name(names, &t["owner_id"]),
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
    if let Some(d) = t["description"].as_str() {
        println!("  about    {d}");
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
        other => other.to_string(),
    }
}

/// Local wall-clock rendering of a ms timestamp.
fn clock(ms: i64, with_date: bool) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms).single() {
        Some(t) if with_date => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        Some(t) => t.format("%H:%M:%S").to_string(),
        None => ms.to_string(),
    }
}
