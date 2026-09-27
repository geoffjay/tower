use clap::{Parser, Subcommand};
use tower_client::Client;

mod task;

#[derive(Debug, Parser)]
#[command(
    name = "tower",
    version,
    about = "control and visibility for agent herds"
)]
pub struct Cli {
    /// Override token (default: TOWER_TOKEN env or token file)
    #[arg(long, global = true)]
    pub token: Option<String>,

    /// Print the raw API JSON instead of a table
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the server
    Serve,
    /// Run a remote-machine node agent (phase 5)
    Node,

    /// List agents with live states
    Ps {
        /// List adoption candidates too
        #[arg(long)]
        all: bool,
    },
    /// Spawn an agent
    Spawn {
        name: String,
        /// Harness kind (pi, claude, ...)
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        workdir: Option<String>,
        /// Isolated git worktree
        #[arg(long)]
        worktree: bool,
        /// Adopt an existing harness agent by name instead of spawning
        #[arg(long)]
        adopt: bool,
        /// First prompt, sent when the agent settles
        #[arg(long)]
        prompt: Option<String>,
    },
    /// Send a prompt to an agent
    Prompt {
        name: String,
        text: String,
        /// Block until the agent reaches a settled state
        #[arg(long)]
        wait: bool,
    },
    /// Read agent terminal output
    Read {
        name: String,
        #[arg(long, default_value = "recent")]
        source: String,
        /// Preserve ANSI colors
        #[arg(long)]
        ansi: bool,
    },
    /// Live-stream an agent's output (SSE)
    Stream { name: String },
    /// Interrupt an agent (ctrl+c)
    Interrupt { name: String },
    /// Stop an agent's session (the seat row survives)
    Stop {
        name: String,
        /// Also remove the agent row
        #[arg(long)]
        remove: bool,
    },
    /// Preflight: herdr, harnesses, database, server
    Doctor,
    /// Route + event-type registry
    Schema,

    /// Job queue: create, assign, list, show, cancel, release
    Task {
        #[command(subcommand)]
        cmd: task::TaskCmd,
    },
    /// Pending questions/approvals addressed to me
    Inbox,
    /// Send a question to an agent
    Ask {
        name: String,
        text: String,
        /// Seconds until the question expires (default 300)
        #[arg(long)]
        deadline_s: Option<i64>,
    },
    /// Answer a question or approval from the inbox
    Approve {
        msg_id: String,
        /// Deny instead of approve
        #[arg(long)]
        deny: bool,
        /// Free-text answer (for questions)
        #[arg(long)]
        answer: Option<String>,
    },
    /// Generic unified send
    Send {
        to: String,
        text: String,
        /// Message kind (prompt, question, notice, ...)
        #[arg(long)]
        kind: Option<String>,
        /// Seconds until questions/approvals expire
        #[arg(long)]
        deadline_s: Option<i64>,
    },
}

#[tokio::main]
async fn main_async() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve => tower_server::serve::serve().await,
        Command::Node => anyhow::bail!("node agent arrives in phase 5"),
        Command::Ps { all } => ps(all, cli.json).await,
        Command::Spawn { .. } => spawn(cli).await,
        Command::Prompt { .. } => prompt(cli).await,
        Command::Read { .. } => read(cli).await,
        Command::Stream { name } => stream(name, cli.token).await,
        Command::Interrupt { name } => interrupt(name).await,
        Command::Stop { .. } => stop(cli).await,
        Command::Doctor => doctor().await,
        Command::Schema => schema().await,
        Command::Inbox => inbox(cli.json).await,
        Command::Task { cmd } => task::run(&client(cli.token).await?, cmd, cli.json).await,
        Command::Ask {
            name,
            text,
            deadline_s,
        } => ask(name, text, deadline_s).await,
        Command::Approve {
            msg_id,
            deny,
            answer,
        } => approve(msg_id, deny, answer).await,
        Command::Send {
            to,
            text,
            kind,
            deadline_s,
        } => send(to, text, kind, deadline_s).await,
    }
}

async fn client(token: Option<String>) -> anyhow::Result<Client> {
    Client::connect(token)
}

async fn ps(all: bool, json_out: bool) -> anyhow::Result<()> {
    let c = client(None).await?;
    let v = c.get("/v1/agents").await?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    let agents = v["agents"].as_array().cloned().unwrap_or_default();
    println!(
        "{:<4} {:<12} {:<8} {:<10} NOTE",
        " ", "NAME", "KIND", "STATE"
    );
    for a in &agents {
        let state = a["state"].as_str().unwrap_or("unknown");
        let glyph = state_glyph(state);
        let name = a["name"].as_str().unwrap_or("?");
        let kind = a["kind"].as_str().unwrap_or("?");
        let mut note = String::new();
        if a["adopted"].as_bool().unwrap_or(false) {
            note.push_str("adopted");
        }
        if let Some(w) = a["worktree"].as_str() {
            if !note.is_empty() {
                note.push_str(" · ");
            }
            note.push_str(w.rsplit('/').next().unwrap_or(w));
        }
        println!("{glyph:<4} {name:<12} {kind:<8} {state:<10} {note}");
    }
    if all {
        let v = c.get("/v1/agents/adoptable").await?;
        let cands = v["adoptable"].as_array().cloned().unwrap_or_default();
        if !cands.is_empty() {
            println!();
            println!("adoptable (not owned):");
            for a in &cands {
                println!(
                    "  {}  {}  {}",
                    a["name"].as_str().unwrap_or("?"),
                    a["kind"].as_str().unwrap_or("?"),
                    a["state"].as_str().unwrap_or("?")
                );
            }
        }
    }
    Ok(())
}

fn state_glyph(state: &str) -> &'static str {
    match state {
        "working" => "●",
        "idle" => "○",
        "blocked" => "◉",
        "done" => "✓",
        "dead" => "✗",
        "launching" => "◌",
        _ => "?",
    }
}

async fn inbox(json_out: bool) -> anyhow::Result<()> {
    let c = client(None).await?;
    let v = c.get("/v1/messages?to=me&status=pending").await?;
    if json_out {
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    let msgs = v["messages"].as_array().cloned().unwrap_or_default();
    if msgs.is_empty() {
        println!("inbox empty");
        return Ok(());
    }
    println!(
        "{:<14} {:<10} {:<18} {:<10} SUMMARY",
        "ID", "FROM", "KIND", "AGE"
    );
    let now = tower_core::now_ms();
    for m in &msgs {
        let id = m["id"].as_str().unwrap_or("?");
        let from = m["from_id"].as_str().unwrap_or("?");
        let kind = m["kind"].as_str().unwrap_or("?");
        let age = age_str(now - m["created_at"].as_i64().unwrap_or(0));
        let summary = m["parts"]
            .as_array()
            .and_then(|p| p.first())
            .and_then(|p| p["text"].as_str())
            .unwrap_or("");
        println!("{:<14} {:<10} {:<18} {:<10} {summary}", id, from, kind, age);
    }
    Ok(())
}

fn age_str(ms: i64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s}s")
    } else {
        format!("{}m", s / 60)
    }
}

async fn ask(name: String, text: String, deadline_s: Option<i64>) -> anyhow::Result<()> {
    let c = client(None).await?;
    let mut body = serde_json::json!({
        "to": name,
        "to_kind": "agent",
        "kind": "question",
        "parts": [{"text": text}],
    });
    if let Some(s) = deadline_s {
        body["deadline_s"] = serde_json::json!(s);
    }
    let v = c.post("/v1/messages", Some(body)).await?;
    let id = v["message"]["id"].as_str().unwrap_or("?");
    println!(
        "question sent → {id} (expires in {})",
        deadline_s.unwrap_or(300)
    );
    Ok(())
}

async fn approve(msg_id: String, deny: bool, answer: Option<String>) -> anyhow::Result<()> {
    let c = client(None).await?;
    // Approvals use `approve` (the server answers the dialog); questions use the answer text.
    let mut body = serde_json::json!({ "approve": !deny });
    if let Some(a) = answer {
        body["parts"] = serde_json::json!([{ "text": a }]);
    }
    let v = c
        .post(&format!("/v1/messages/{msg_id}/respond"), Some(body))
        .await?;
    let m = &v["message"];
    let verb = match (m["kind"].as_str(), deny) {
        (Some("approval"), false) => "approved",
        (Some("approval"), true) => "denied",
        _ => "answered",
    };
    println!(
        "{verb} → delivered to {}",
        m["from_id"].as_str().unwrap_or("?")
    );
    Ok(())
}

async fn send(
    to: String,
    text: String,
    kind: Option<String>,
    deadline_s: Option<i64>,
) -> anyhow::Result<()> {
    let c = client(None).await?;
    let mut body = serde_json::json!({
        "to": to,
        "parts": [{"text": text}],
    });
    if let Some(k) = kind {
        body["kind"] = serde_json::json!(k);
    }
    if let Some(s) = deadline_s {
        body["deadline_s"] = serde_json::json!(s);
    }
    let v = c.post("/v1/messages", Some(body)).await?;
    let id = v["message"]["id"].as_str().unwrap_or("?");
    println!("sent → {id}");
    Ok(())
}

async fn spawn(cli: Cli) -> anyhow::Result<()> {
    let Command::Spawn {
        name,
        kind,
        workdir,
        worktree,
        adopt,
        prompt,
    } = cli.command
    else {
        unreachable!()
    };
    let c = client(cli.token).await?;
    let mut body = serde_json::json!({"name": name, "adopt": adopt});
    if let Some(k) = kind {
        body["kind"] = serde_json::json!(k);
    }
    if let Some(w) = workdir {
        body["workdir"] = serde_json::json!(w);
    }
    if worktree {
        body["worktree"] = serde_json::json!(true);
    }
    if let Some(p) = prompt {
        body["prompt"] = serde_json::json!(p);
    }
    let v = c.post("/v1/agents", Some(body)).await?;
    let a = &v["agent"];
    println!(
        "spawned {} ({})",
        a["name"].as_str().unwrap_or("?"),
        a["kind"].as_str().unwrap_or("?")
    );
    if let Some(p) = a["worktree"].as_str() {
        println!("  worktree {p}");
    }
    println!("  state    {}", a["state"].as_str().unwrap_or("launching"));
    Ok(())
}

async fn prompt(cli: Cli) -> anyhow::Result<()> {
    let Command::Prompt { name, text, wait } = cli.command else {
        unreachable!()
    };
    let c = client(cli.token).await?;
    let v = c
        .post(
            &format!("/v1/agents/{name}/prompt"),
            Some(serde_json::json!({"text": text, "wait": wait})),
        )
        .await?;
    let state = v["state"].as_str().unwrap_or("unknown");
    if v["stalled"] == true {
        println!(
            "prompt delivered; herdr saw no state change within 5s (now {state}) — \
             the agent may have answered instantly; check `tower read {name}`"
        );
    } else if wait {
        println!("prompt delivered; settled in {state}");
    } else {
        println!("prompt delivered");
    }
    Ok(())
}

async fn read(cli: Cli) -> anyhow::Result<()> {
    let Command::Read { name, source, ansi } = cli.command else {
        unreachable!()
    };
    let c = client(cli.token).await?;
    let path = if ansi {
        format!("/v1/agents/{name}/read?format=ansi")
    } else {
        format!("/v1/agents/{name}/read")
    };
    let _ = source; // v1: server picks recent/visible; per-source arrives with agent detail views
    let v = c.get(&path).await?;
    print!("{}", v["output"].as_str().unwrap_or(""));
    Ok(())
}

async fn stream(name: String, token: Option<String>) -> anyhow::Result<()> {
    let c = client(token).await?;
    // events carry the agent id as subject; resolve the name once
    let id = c.get(&format!("/v1/agents/{name}")).await?["agent"]["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("agent {name} not found"))?
        .to_string();
    // seed with a read, then follow the SSE bus filtered to this agent
    let v = c.get(&format!("/v1/agents/{name}/read")).await?;
    print!("{}", v["output"].as_str().unwrap_or(""));
    let resp = c
        .stream(&format!("/v1/events?subject=agent:{id}&filter=type:agent"))
        .await?;
    use futures::StreamExt;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let text = String::from_utf8_lossy(&chunk);
        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if v["kind"] == "agent.output" {
                        if let Some(out) = v["payload"]["text"].as_str() {
                            print!("{out}");
                        }
                    }
                }
            }
        }
        use std::io::Write;
        std::io::stdout().flush().ok();
    }
    Ok(())
}

async fn interrupt(name: String) -> anyhow::Result<()> {
    let c = client(None).await?;
    c.post(&format!("/v1/agents/{name}/interrupt"), None)
        .await?;
    println!("interrupted {name}");
    Ok(())
}

async fn stop(cli: Cli) -> anyhow::Result<()> {
    let Command::Stop { name, remove } = cli.command else {
        unreachable!()
    };
    let c = client(cli.token).await?;
    c.post(
        &format!("/v1/agents/{name}/stop"),
        Some(serde_json::json!({"remove": remove})),
    )
    .await?;
    println!("stopped {name}");
    Ok(())
}

async fn doctor() -> anyhow::Result<()> {
    let mut checks = 0;
    let mut failed = 0;

    macro_rules! check {
        ($name:expr, $ok:expr, $detail:expr) => {{
            checks += 1;
            if $ok {
                println!("{:<16} ok    {}", $name, $detail);
            } else {
                failed += 1;
                println!("{:<16} FAIL  {}", $name, $detail);
            }
        }};
    }

    // herdr
    let herdr_ok = std::process::Command::new("herdr")
        .arg("status")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    check!("herdr server", herdr_ok, "herdr status");

    // harnesses on PATH
    for harness in ["pi", "claude"] {
        let ok = which(harness);
        let detail = if ok { "found" } else { "not found" };
        check!(harness, ok, detail);
    }

    // tower server reachable: report the real cause (missing token file,
    // connection refused, 401), not a generic "not reachable"
    let server = match Client::connect(None) {
        Ok(c) => c.get("/healthz").await.map(|v| v["ok"] == true),
        Err(e) => Err(e),
    };
    match server {
        Ok(true) => check!("tower server", true, "healthz ok"),
        Ok(false) => check!("tower server", false, "healthz reports a database error"),
        Err(e) => {
            let mut detail = format!("{e:#}");
            if detail.contains("token file") {
                let home = std::env::var("TOWER_HOME").unwrap_or_else(|_| "(default)".into());
                detail.push_str(&format!(
                    " — TOWER_HOME={home}; it must match the server's (`tower serve` prints its token path)"
                ));
            }
            check!("tower server", false, detail)
        }
    }

    println!();
    if failed == 0 {
        println!("{checks} checks passed");
    } else {
        println!("{checks} checks, {failed} failed");
        std::process::exit(1);
    }
    Ok(())
}

fn which(bin: &str) -> bool {
    std::env::var("PATH")
        .map(|p| {
            p.split(':')
                .any(|dir| std::path::Path::new(dir).join(bin).exists())
        })
        .unwrap_or(false)
}

async fn schema() -> anyhow::Result<()> {
    let c = client(None).await?;
    let v = c.get("/v1/schema").await?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}

pub fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();
    main_async()
}
