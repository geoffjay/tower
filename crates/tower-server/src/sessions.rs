//! Sessions module (D§9.2, plan T5.2): spawn/prompt/interrupt/stop flow.
//! Driver calls are serialized per agent (D§9.2).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sqlx::Row;
use tower_core::{Agent, AgentId, AgentState, MachineId, TowerError};
use tower_driver::{AgentSpec, ReadSource};

use crate::state::AppState;

/// Serialize driver calls per agent name (herdr prompts wait on detection
/// anyway; prevents concurrent prompt/spawn races on one pane).
type SharedMutex = Arc<tokio::sync::Mutex<()>>;

pub fn lock_for(agent: &str) -> SharedMutex {
    use std::sync::OnceLock;
    static LOCKS: OnceLock<Mutex<HashMap<String, SharedMutex>>> = OnceLock::new();
    let map = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    map.lock()
        .unwrap()
        .entry(agent.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

#[derive(serde::Deserialize)]
pub struct SpawnRequest {
    pub name: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub workdir: Option<String>,
    #[serde(default)]
    pub worktree: bool,
    #[serde(default)]
    pub permissions: Option<String>,
    #[serde(default)]
    pub adopt: bool,
    #[serde(default)]
    pub prompt: Option<String>,
}

pub async fn spawn(state: &AppState, req: SpawnRequest) -> anyhow::Result<Agent> {
    if req.adopt {
        return crate::inventory::adopt(state, &req.name).await;
    }

    let kind = req.kind.clone().unwrap_or_else(|| "pi".into());

    let guard = lock_for(&req.name);
    let _guard = guard.lock().await;

    if crate::inventory::get_agent(state, &req.name)
        .await?
        .is_some()
    {
        anyhow::bail!("agent {} already exists", req.name);
    }

    // spawn via driver; the pane learns who it is (CLI `--as` default, D§7)
    // and where this server's data dir is when non-default
    let mut env = vec![("TOWER_AGENT".to_string(), req.name.clone())];
    if let Ok(home) = std::env::var("TOWER_HOME") {
        env.push(("TOWER_HOME".into(), home));
    }
    let spec = AgentSpec {
        name: req.name.clone(),
        kind: kind.clone(),
        workdir: req.workdir.clone(),
        args: vec![],
        env,
    };
    let pane = state.driver.start(&spec).await?;

    let id = AgentId::new();
    let machine = crate::inventory::ensure_local_machine(state).await?;
    let now = tower_core::now_ms();
    let perms = req.permissions.unwrap_or_else(|| "default".into());

    // worktree isolation (best-effort; workdir must be a git repo)
    let worktree = if req.worktree {
        match make_worktree(req.workdir.as_deref()) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::warn!(error = %e, "worktree creation failed; using workdir");
                None
            }
        }
    } else {
        None
    };

    sqlx::query(
        "INSERT INTO agents (id, name, kind, machine_id, pane_id, workdir, worktree, state,
         desired_state, permissions, adopted, config, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'launching', 'running', ?8, 0, '{}', ?9, ?9)",
    )
    .bind(id.0.as_str())
    .bind(&req.name)
    .bind(&req.kind)
    .bind(machine.0.as_str())
    .bind(&pane)
    .bind(req.workdir.as_deref())
    .bind(worktree.as_deref())
    .bind(&perms)
    .bind(now)
    .execute(&state.pool)
    .await?;

    state
        .events
        .append(
            tower_core::EventKind::AgentCreated,
            Some("agent"),
            Some(&id.0),
            serde_json::json!({"name": req.name, "kind": kind, "pane": pane,
                               "permissions": perms}),
        )
        .await?;

    // first prompt, if requested
    if let Some(text) = req.prompt {
        let _ = prompt(state, &req.name, &text, false).await;
    }

    // refresh state from the harness (launching → idle typically)
    let _ = crate::inventory::reconcile(state).await;
    Ok(crate::inventory::get_agent(state, &req.name)
        .await?
        .expect("just inserted"))
}

fn make_worktree(workdir: Option<&str>) -> anyhow::Result<String> {
    let dir = workdir.ok_or_else(|| anyhow::anyhow!("workdir required for worktree"))?;
    let name = format!(".worktrees/{}", tower_core::new_id().to_lowercase());
    let out = std::process::Command::new("git")
        .args(["worktree", "add", "--detach"])
        .arg(&name)
        .current_dir(dir)
        .output()
        .map_err(|e| anyhow::anyhow!("git worktree add: {e}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(format!("{dir}/{name}"))
}

#[derive(serde::Deserialize)]
pub struct PromptRequest {
    pub text: String,
    #[serde(default)]
    pub wait: bool,
}

/// What a delivered prompt led to.
#[derive(Debug, serde::Serialize)]
pub struct PromptOutcome {
    /// Agent state after delivery (harness truth, post-reconcile).
    pub state: AgentState,
    /// `--wait` saw no state change within herdr's 5s window. The prompt WAS
    /// delivered (S1.A: not fatal) — the agent may have answered faster
    /// than detection, or be at an error screen; read its output.
    pub stalled: bool,
}

pub async fn prompt(
    state: &AppState,
    name: &str,
    text: &str,
    wait: bool,
) -> anyhow::Result<PromptOutcome> {
    let agent = crate::inventory::get_agent(state, name)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("agent {name} not found")))?;
    let guard = lock_for(name);
    let _guard = guard.lock().await;
    let stalled = match state.driver.prompt(&agent.name, text, wait).await {
        Ok(()) => false,
        Err(tower_driver::DriverError::PromptStalled(_)) => true,
        Err(e) => return Err(e.into()),
    };
    state
        .events
        .append(
            tower_core::EventKind::MessageCreated,
            Some("agent"),
            Some(&agent.id.0),
            serde_json::json!({"kind": "prompt", "text": text, "wait": wait}),
        )
        .await?;
    // refresh state from harness truth (prompt often flips working)
    let _ = crate::inventory::reconcile(state).await;
    let now = crate::inventory::get_agent(state, &agent.name)
        .await?
        .map(|a| a.state)
        .unwrap_or(agent.state);
    Ok(PromptOutcome {
        state: now,
        stalled,
    })
}

pub async fn interrupt(state: &AppState, name: &str) -> anyhow::Result<()> {
    let agent = crate::inventory::get_agent(state, name)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("agent {name} not found")))?;
    state.driver.interrupt(&agent.name).await?;
    Ok(())
}

pub async fn read(state: &AppState, name: &str, ansi: bool) -> anyhow::Result<String> {
    let agent = crate::inventory::get_agent(state, name)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("agent {name} not found")))?;
    let source = if ansi {
        ReadSource::Visible
    } else {
        ReadSource::Recent
    };
    Ok(state.driver.read(&agent.name, source, ansi).await?.text)
}

pub async fn stop(state: &AppState, name: &str, remove: bool) -> anyhow::Result<()> {
    let agent = crate::inventory::get_agent(state, name)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("agent {name} not found")))?;
    let guard = lock_for(name);
    let _guard = guard.lock().await;
    // pane first: an agent must be gone before its jobs are requeued, or it
    // could keep working a job someone else is then assigned. A pane that's
    // already gone counts as stopped (removing a dead agent must work).
    match state
        .driver
        .stop(&agent.name, agent.pane_id.as_deref())
        .await
    {
        Ok(()) | Err(tower_driver::DriverError::NotFound(_)) => {}
        Err(e) => return Err(e.into()),
    }

    if remove {
        crate::tasks::detach_agent(state, &agent, tower_core::now_ms()).await?;
        sqlx::query("DELETE FROM agents WHERE id = ?1")
            .bind(agent.id.0.as_str())
            .execute(&state.pool)
            .await?;
        state
            .events
            .append(
                tower_core::EventKind::AgentRemoved,
                Some("agent"),
                Some(&agent.id.0),
                serde_json::json!({"name": agent.name}),
            )
            .await?;
    } else {
        sqlx::query("UPDATE agents SET state='dead', pane_id=NULL, updated_at=?1 WHERE id=?2")
            .bind(tower_core::now_ms())
            .bind(agent.id.0.as_str())
            .execute(&state.pool)
            .await?;
        state
            .events
            .append(
                tower_core::EventKind::AgentStateChange,
                Some("agent"),
                Some(&agent.id.0),
                serde_json::json!({"from": agent.state.as_str(), "to": "dead"}),
            )
            .await?;
    }
    Ok(())
}

/// Row → Agent for the sessions API responses.
pub fn row_to_agent(r: &sqlx::sqlite::SqliteRow) -> Agent {
    let state_str: String = r.get("state");
    Agent {
        id: AgentId::from(r.get::<String, _>("id")),
        name: r.get("name"),
        kind: r.get("kind"),
        machine_id: MachineId::from(r.get::<String, _>("machine_id")),
        pane_id: r.try_get("pane_id").ok(),
        workdir: r.try_get("workdir").ok(),
        worktree: r.try_get("worktree").ok(),
        state: serde_json::from_value(serde_json::Value::String(state_str))
            .unwrap_or(AgentState::Unknown),
        desired_state: tower_core::DesiredState::Running,
        permissions: tower_core::Permissions::Default,
        adopted: r.get::<i64, _>("adopted") != 0,
        config: serde_json::json!({}),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}
