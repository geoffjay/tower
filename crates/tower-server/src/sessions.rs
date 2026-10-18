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
    /// Relaunch a `dead` row's harness with a given session; an empty
    /// string means "the session herdr reports for the pane".
    #[serde(default)]
    pub resume: Option<String>,
    /// Relaunch a `dead` row's harness continuing its last session.
    #[serde(default, rename = "continue")]
    pub continue_last: bool,
    #[serde(default)]
    pub prompt: Option<String>,
    /// Model the harness runs on; passed as `--model <id>` (all harnesses
    /// take the same flag).
    #[serde(default)]
    pub model: Option<String>,
    /// Extra harness flags (definition `args` or `tower spawn --arg`).
    #[serde(default)]
    pub args: Vec<String>,
}

pub async fn spawn(state: &AppState, req: SpawnRequest) -> anyhow::Result<Agent> {
    if req.adopt {
        return crate::inventory::adopt(state, &req.name).await;
    }
    if req.resume.is_some() || req.continue_last {
        return resume(state, &req).await;
    }

    let kind = req.kind.clone().unwrap_or_else(|| "pi".into());

    let guard = lock_for(&req.name);
    let creating = guard.lock().await;

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
    // Harness args (D§4): per-kind defaults from config.toml, then the
    // agent's own args, then `--model <id>`. All reach the harness after
    // `--` in `herdr agent start` (D§8).
    let mut args: Vec<String> = state.config.harnesses.args_for(&kind).to_vec();
    args.extend(req.args.iter().cloned());
    if let Some(m) = &req.model {
        args.push("--model".into());
        args.push(m.clone());
    }

    // worktree isolation (D§9.2): made before the harness starts, which then
    // runs in it; no fallback to the main checkout when it can't be made
    let worktree = if req.worktree {
        Some(make_worktree(req.workdir.as_deref(), &req.name)?)
    } else {
        None
    };

    let spec = AgentSpec {
        name: req.name.clone(),
        kind: kind.clone(),
        workdir: worktree.clone().or_else(|| req.workdir.clone()),
        args,
        env,
        pane: None,
    };
    let pane = state.driver.start(&spec).await?;

    let id = AgentId::new();
    let machine = crate::inventory::ensure_local_machine(state).await?;
    let now = tower_core::now_ms();
    let perms = req.permissions.unwrap_or_else(|| "default".into());

    // per-agent args only (the per-kind [harness.<kind>] args defaults are
    // applied at spawn, not stored — they are config, not row state)
    let agent_args: String = serde_json::to_string(&req.args).unwrap_or_else(|_| "[]".into());
    sqlx::query(
        "INSERT INTO agents (id, name, kind, machine_id, pane_id, workdir, worktree, state,
         desired_state, permissions, adopted, config, model, args, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'launching', 'running', ?8, 0, '{}', ?9, ?10, ?11, ?11)",
    )
    .bind(id.0.as_str())
    .bind(&req.name)
    .bind(&req.kind)
    .bind(machine.0.as_str())
    .bind(&pane)
    .bind(req.workdir.as_deref())
    .bind(worktree.as_deref())
    .bind(&perms)
    .bind(req.model.as_deref())
    .bind(&agent_args)
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
                               "permissions": perms, "model": req.model}),
        )
        .await?;

    // release before anything that locks this agent again (the first prompt,
    // and reconcile → dispatch → delivery prompt): tokio locks don't re-enter
    drop(creating);

    // first prompt, if requested
    if let Some(text) = req.prompt {
        let _ = prompt(state, &req.name, &text, false, OPERATOR).await;
    }

    // refresh state from the harness (launching → idle typically)
    let _ = crate::inventory::reconcile(state).await;
    Ok(crate::inventory::get_agent(state, &req.name)
        .await?
        .expect("just inserted"))
}

/// Relaunch a `dead` row's harness with its previous session (issue #1):
/// `herdr agent start` in the row's pane — a new pane if it is gone —
/// with the per-kind resume args (D§8.4). The row keeps its id, jobs,
/// and schedules; only pane and state move.
async fn resume(state: &AppState, req: &SpawnRequest) -> anyhow::Result<Agent> {
    let row = crate::inventory::get_agent(state, &req.name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("agent {} does not exist", req.name))?;
    if row.state != tower_core::AgentState::Dead {
        anyhow::bail!(
            "agent {} is {} — resume needs a dead row",
            req.name,
            row.state.as_str()
        );
    }
    let kind = row.kind.clone();

    // which session: explicit `resume`, else continue, else the session
    // herdr last reported for the pane
    let args: Vec<String> = if let Some(session) = &req.resume {
        let session = if session.is_empty() {
            let reported = match row.pane_id.as_deref() {
                Some(p) => state.driver.agent_at(p).await?,
                None => None,
            };
            reported.and_then(|a| a.session).ok_or_else(|| {
                anyhow::anyhow!(
                    "no session reported for {}'s pane; pass --resume <session>",
                    req.name
                )
            })?
        } else {
            session.clone()
        };
        match kind.as_str() {
            "pi" => vec!["--session".into(), session],
            _ => vec!["--resume".into(), session], // omp, claude
        }
    } else if req.continue_last {
        vec!["--continue".into()]
    } else {
        Vec::new()
    };

    let guard = lock_for(&req.name);
    let creating = guard.lock().await;

    let mut env = vec![("TOWER_AGENT".to_string(), req.name.clone())];
    if let Ok(home) = std::env::var("TOWER_HOME") {
        env.push(("TOWER_HOME".into(), home));
    }
    let spec = AgentSpec {
        name: req.name.clone(),
        kind: kind.clone(),
        // an isolated agent relaunches in its worktree (D§9.2)
        workdir: row.worktree.clone().or_else(|| row.workdir.clone()),
        args,
        env,
        pane: row.pane_id.clone(),
    };
    // the row's pane if it still exists, else a fresh one (D§8.4)
    let pane = match state.driver.start(&spec).await {
        Ok(p) => p,
        Err(tower_driver::DriverError::NotFound(_)) => {
            let spec = AgentSpec { pane: None, ..spec };
            state.driver.start(&spec).await?
        }
        Err(e) => return Err(e.into()),
    };

    // the row keeps its identity; pane and liveness move
    sqlx::query(
        "UPDATE agents SET pane_id = ?1, state = 'launching', updated_at = ?2
                 WHERE id = ?3",
    )
    .bind(&pane)
    .bind(tower_core::now_ms())
    .bind(row.id.0.as_str())
    .execute(&state.pool)
    .await?;

    drop(creating);
    if let Some(text) = &req.prompt {
        let _ = prompt(state, &req.name, text, false, OPERATOR).await;
    }
    let _ = crate::inventory::reconcile(state).await;
    Ok(crate::inventory::get_agent(state, &req.name)
        .await?
        .expect("row exists"))
}

/// Run git in `dir`: stdout, or an error carrying git's own message.
fn git(dir: &str, args: &[&str]) -> anyhow::Result<String> {
    let what = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| anyhow::anyhow!("git {what}: {e}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "git {what}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The agent's worktree, `<workdir>/.worktrees/<name>` (D§9.2): reused when
/// it is already a worktree of the workdir's repository (a re-created agent
/// keeps its files and branches), else added on a detached HEAD at the
/// workdir's current commit. Errors carry git's message.
fn make_worktree(workdir: Option<&str>, name: &str) -> anyhow::Result<String> {
    let dir =
        workdir.ok_or_else(|| anyhow::anyhow!("--worktree needs a workdir (a git checkout)"))?;
    // the name becomes one path component under .worktrees/
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        anyhow::bail!("agent name {name:?} can't name a worktree directory");
    }
    let path = format!("{}/.worktrees/{name}", dir.trim_end_matches('/'));
    if is_worktree_of(dir, &path)? {
        return Ok(path);
    }
    git(dir, &["worktree", "add", "--detach", &path])?;
    Ok(path)
}

/// Whether `path` is a registered worktree of `dir`'s repository (paths
/// compared canonically: macOS reports /tmp as /private/tmp).
fn is_worktree_of(dir: &str, path: &str) -> anyhow::Result<bool> {
    let Ok(want) = std::fs::canonicalize(path) else {
        return Ok(false);
    };
    let list = git(dir, &["worktree", "list", "--porcelain"])?;
    Ok(list
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .any(|p| std::fs::canonicalize(p).is_ok_and(|p| p == want)))
}

/// What `stop --remove` did with the agent's worktree (D§9.2).
#[derive(Debug, serde::Serialize)]
pub struct WorktreeCleanup {
    pub path: String,
    pub removed: bool,
    /// Why it was kept: git's message (modified or untracked files, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `git worktree remove` without `--force`: git itself refuses a worktree
/// with modified or untracked files, and that refusal is the reason kept.
fn remove_worktree(workdir: Option<&str>, path: &str) -> WorktreeCleanup {
    let kept = |reason: String| WorktreeCleanup {
        path: path.to_string(),
        removed: false,
        reason: Some(reason),
    };
    // run from the main checkout, not from inside the worktree itself
    let Some(dir) = workdir else {
        return kept("no workdir recorded for this agent".into());
    };
    match git(dir, &["worktree", "remove", path]) {
        Ok(_) => WorktreeCleanup {
            path: path.to_string(),
            removed: true,
            reason: None,
        },
        Err(e) => kept(e.to_string()),
    }
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

/// The operator as a message party (`from` of operator prompts).
pub const OPERATOR: (tower_core::PartyKind, &str) = (tower_core::PartyKind::Human, "me");

/// Prompt an agent (D§7): drive the harness, then record the `prompt`
/// message row so it shows in the agent's history (`?agent=`, D§11.1).
pub async fn prompt(
    state: &AppState,
    name: &str,
    text: &str,
    wait: bool,
    from: (tower_core::PartyKind, &str),
) -> anyhow::Result<PromptOutcome> {
    let agent = crate::inventory::get_agent(state, name)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("agent {name} not found")))?;
    let guard = lock_for(name);
    let prompting = guard.lock().await;
    let stalled = match state.driver.prompt(&agent.name, text, wait).await {
        Ok(()) => false,
        Err(tower_driver::DriverError::PromptStalled(_)) => true,
        Err(e) => return Err(e.into()),
    };
    // reconcile below can dispatch a reserved job to this agent, which
    // prompts it — that takes this lock again
    drop(prompting);
    crate::messaging::insert(
        state,
        crate::messaging::NewMessage {
            task_id: None,
            from_kind: from.0,
            from_id: from.1,
            to_kind: tower_core::PartyKind::Agent,
            to_id: &agent.name,
            kind: tower_core::MessageKind::Prompt,
            parts: &[tower_core::Part::text(text)],
            deadline_s: None,
        },
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

/// What `stop` did beyond ending the session.
#[derive(Debug, Default)]
pub struct StopOutcome {
    /// `remove` on an agent with a worktree: removed, or kept and why.
    pub worktree: Option<WorktreeCleanup>,
}

pub async fn stop(state: &AppState, name: &str, remove: bool) -> anyhow::Result<StopOutcome> {
    let agent = crate::inventory::get_agent(state, name)
        .await?
        .ok_or_else(|| TowerError::not_found(format!("agent {name} not found")))?;
    let guard = lock_for(name);
    let stopping = guard.lock().await;
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
    // the lock serializes driver calls; the DB cleanup below must not hold
    // it (releasing jobs can dispatch → prompt, which takes this lock)
    drop(stopping);

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
        // after the pane is gone, so nothing runs in the worktree any more
        let worktree = agent
            .worktree
            .as_deref()
            .map(|path| remove_worktree(agent.workdir.as_deref(), path));
        return Ok(StopOutcome { worktree });
    }
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
    Ok(StopOutcome::default())
}

/// Row → Agent for the sessions API responses.
pub fn row_to_agent(r: &sqlx::sqlite::SqliteRow) -> Agent {
    let state_str: String = r.get("state");
    Agent {
        id: AgentId::from(r.get::<String, _>("id")),
        name: r.get("name"),
        kind: r.get("kind"),
        machine_id: MachineId::from(r.get::<String, _>("machine_id")),
        pane_id: r.try_get::<Option<String>, _>("pane_id").ok().flatten(),
        workdir: r.try_get::<Option<String>, _>("workdir").ok().flatten(),
        worktree: r.try_get::<Option<String>, _>("worktree").ok().flatten(),
        state: serde_json::from_value(serde_json::Value::String(state_str))
            .unwrap_or(AgentState::Unknown),
        desired_state: tower_core::DesiredState::Running,
        permissions: tower_core::Permissions::Default,
        adopted: r.get::<i64, _>("adopted") != 0,
        config: serde_json::json!({}),
        model: r.try_get::<Option<String>, _>("model").ok().flatten(),
        args: serde_json::from_str(&r.get::<String, _>("args")).unwrap_or_default(),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}
