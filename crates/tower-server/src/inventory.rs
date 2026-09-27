//! Inventory module (D§9.1, plan T5.1): agents + machines rows, boot
//! reconcile against the driver snapshot, adoption candidates.

use sqlx::Row;
use tower_core::{Agent, AgentId, AgentState, MachineId};
use tower_driver::HarnessState;

use crate::state::AppState;

/// Ensure the `local` machine row exists (boot sequence step 1).
pub async fn ensure_local_machine(state: &AppState) -> anyhow::Result<MachineId> {
    let id = MachineId::from("local");
    sqlx::query(
        "INSERT INTO machines (id, name, role, address, status, last_seen_at, created_at)
         VALUES (?1, 'local', 'coordinator', NULL, 'online', ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET status='online', last_seen_at=?2",
    )
    .bind(id.0.as_str())
    .bind(tower_core::now_ms())
    .bind(tower_core::now_ms())
    .execute(&state.pool)
    .await?;
    Ok(id)
}

fn state_row_to_agent(r: &sqlx::sqlite::SqliteRow) -> Agent {
    let state_str: String = r.get("state");
    let desired: String = r.get("desired_state");
    let perms: String = r.get("permissions");
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
        desired_state: serde_json::from_value(serde_json::Value::String(desired))
            .unwrap_or(tower_core::DesiredState::Running),
        permissions: serde_json::from_value(serde_json::Value::String(perms))
            .unwrap_or(tower_core::Permissions::Default),
        adopted: r.get::<i64, _>("adopted") != 0,
        config: serde_json::from_str(&r.get::<String, _>("config")).unwrap_or_default(),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

pub async fn list_agents(state: &AppState) -> anyhow::Result<Vec<Agent>> {
    let rows = sqlx::query("SELECT * FROM agents ORDER BY created_at")
        .fetch_all(&state.pool)
        .await?;
    Ok(rows.iter().map(state_row_to_agent).collect())
}

/// Machine inventory (D§5.5); `local` is always present.
pub async fn list_machines(state: &AppState) -> anyhow::Result<Vec<tower_core::Machine>> {
    let rows = sqlx::query("SELECT * FROM machines ORDER BY created_at")
        .fetch_all(&state.pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| tower_core::Machine {
            id: MachineId::from(r.get::<String, _>("id")),
            name: r.get("name"),
            role: r.get("role"),
            address: r.get("address"),
            status: r.get("status"),
            last_seen_at: r.get("last_seen_at"),
            created_at: r.get("created_at"),
        })
        .collect())
}

pub async fn get_agent(state: &AppState, name_or_id: &str) -> anyhow::Result<Option<Agent>> {
    let rows = sqlx::query("SELECT * FROM agents WHERE id = ?1 OR name = ?1")
        .bind(name_or_id)
        .fetch_all(&state.pool)
        .await?;
    Ok(rows.first().map(state_row_to_agent))
}

/// Boot reconcile (D§9.1): driver snapshot vs agents table.
///
/// - rows whose pane still exists: refresh state, emit drift events
/// - rows whose pane is gone (and not adopted-only): mark dead, emit event
/// - harness agents with no row: adoption candidates (listed, NOT owned
///   until `adopt` — openrig's discover/adopt split, D§8.4)
pub async fn reconcile(state: &AppState) -> anyhow::Result<()> {
    let snap = state.driver.snapshot().await?;
    let rows = list_agents(state).await?;

    // refresh + dead detection for owned rows
    for agent in &rows {
        let live = snap.iter().find(|s| {
            s.pane_id == agent.pane_id.clone().unwrap_or_default() || s.name == agent.name
        });
        match live {
            Some(h) => {
                transition(state, agent, map_harness_state(h.state)).await?;
            }
            None => {
                if agent.pane_id.is_some() {
                    transition(state, agent, AgentState::Dead).await?;
                }
            }
        }
    }

    // adoption candidates: harness agents with no tower row
    for h in &snap {
        let owned = rows
            .iter()
            .any(|a| a.pane_id.as_deref() == Some(h.pane_id.as_str()) || a.name == h.name);
        if !owned {
            tracing::info!(name = %h.name, kind = %h.kind, "adoption candidate");
            // listed via `adoption_candidates`; ownership requires POST adopt
        }
    }
    Ok(())
}

/// Harness agents not owned by tower rows (D§8.4).
pub async fn adoption_candidates(state: &AppState) -> anyhow::Result<Vec<serde_json::Value>> {
    let snap = state.driver.snapshot().await?;
    let rows = list_agents(state).await?;
    Ok(snap
        .iter()
        .filter(|h| {
            !rows
                .iter()
                .any(|a| a.pane_id.as_deref() == Some(h.pane_id.as_str()) || a.name == h.name)
        })
        .map(|h| {
            serde_json::json!({
                "name": h.name,
                "kind": h.kind,
                "pane_id": h.pane_id,
                "state": h.state.as_str(),
            })
        })
        .collect())
}

/// Adopt: take ownership of a harness agent without relaunching.
pub async fn adopt(state: &AppState, name: &str) -> anyhow::Result<Agent> {
    let snap = state.driver.snapshot().await?;
    let h = snap
        .iter()
        .find(|h| h.name == name)
        .ok_or_else(|| anyhow::anyhow!("no harness agent named {name}"))?;

    let existing = get_agent(state, name).await?;
    if existing.is_some() {
        anyhow::bail!("agent row {name} already exists");
    }

    let id = AgentId::new();
    let now = tower_core::now_ms();
    let kind = h.kind.clone();
    let pane = h.pane_id.clone();
    let machine = ensure_local_machine(state).await?;
    let state_str = map_harness_state(h.state).as_str();

    sqlx::query(
        "INSERT INTO agents (id, name, kind, machine_id, pane_id, workdir, worktree, state,
         desired_state, permissions, adopted, config, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6, 'running', 'default', 1, '{}', ?7, ?7)",
    )
    .bind(id.0.as_str())
    .bind(name)
    .bind(&kind)
    .bind(machine.0.as_str())
    .bind(&pane)
    .bind(state_str)
    .bind(now)
    .execute(&state.pool)
    .await?;

    state
        .events
        .append(
            tower_core::EventKind::AgentCreated,
            Some("agent"),
            Some(&id.0),
            serde_json::json!({"name": name, "kind": kind, "adopted": true}),
        )
        .await?;

    Ok(get_agent(state, name).await?.expect("just inserted"))
}

/// The one place an agent row changes state (D§5.1). CAS on the observed
/// state, so when reconcile and the pump see the same change only one
/// transition happens. On success: `agent.state` event, the owner's job
/// follows (`blocked` ↔ `input-required`, D§5.2), and entering `blocked`
/// opens the operator's inbox item (D§9.3). Returns whether it changed.
pub async fn transition(state: &AppState, agent: &Agent, new: AgentState) -> anyhow::Result<bool> {
    if agent.state == new {
        return Ok(false);
    }
    let now = tower_core::now_ms();
    let res =
        sqlx::query("UPDATE agents SET state = ?1, updated_at = ?2 WHERE id = ?3 AND state = ?4")
            .bind(new.as_str())
            .bind(now)
            .bind(agent.id.0.as_str())
            .bind(agent.state.as_str())
            .execute(&state.pool)
            .await?;
    if res.rows_affected() == 0 {
        return Ok(false); // someone else already moved it
    }
    state
        .events
        .append(
            tower_core::EventKind::AgentStateChange,
            Some("agent"),
            Some(&agent.id.0),
            serde_json::json!({"from": agent.state.as_str(), "to": new.as_str()}),
        )
        .await?;
    crate::tasks::sync_agent_state(state, &agent.id, new, now).await?;
    // an agent that just became free takes its next reserved job (D§5.2.2)
    if matches!(new, AgentState::Idle | AgentState::Done) {
        crate::tasks::dispatch_for(state, &agent.id, now).await?;
    }
    if new == AgentState::Blocked {
        if let Err(e) = crate::messaging::inbox_for_blocked(state, agent).await {
            tracing::warn!(error = %e, agent = %agent.name, "blocked → inbox failed");
        }
    }
    Ok(true)
}

pub fn map_harness_state(s: HarnessState) -> AgentState {
    match s {
        HarnessState::Idle => AgentState::Idle,
        HarnessState::Working => AgentState::Working,
        HarnessState::Blocked => AgentState::Blocked,
        HarnessState::Done => AgentState::Done,
        HarnessState::Unknown => AgentState::Unknown,
    }
}
