//! Driver event pump (D§9.2/9.3, plan T2.1): consumes the harness event
//! stream and feeds owned agents' state changes through
//! `inventory::transition` (the single place a row changes state — it emits
//! `agent.state`, maps the owner's job, and opens the inbox item on
//! `blocked`), plus `agent.output` chunks.
//!
//! Harness agents without a tower row (adoption candidates) are ignored.

use tower_driver::HarnessEvent;

use crate::inventory;
use crate::state::AppState;

/// Spawn the pump task. Runs until the server exits; transient driver
/// errors inside the stream are swallowed (the driver retries polling).
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    let mut events = state.driver.events();
    let st = state.clone();
    tokio::spawn(async move {
        while let Some(ev) = futures::StreamExt::next(&mut events).await {
            if let Err(e) = handle(&st, ev).await {
                tracing::warn!(error = %e, "pump: event handling failed");
            }
        }
    })
}

async fn handle(state: &AppState, ev: HarnessEvent) -> anyhow::Result<()> {
    let (name, new_state) = match ev {
        // first sighting counts too: an agent already blocked when seen
        // (at spawn, at server start) still gets its inbox item
        HarnessEvent::AgentUp(a) => (a.name, inventory::map_harness_state(a.state)),
        HarnessEvent::StateChange { name, to, .. } => (name, inventory::map_harness_state(to)),
        HarnessEvent::AgentDown { name, .. } => (name, tower_core::AgentState::Dead),
        HarnessEvent::Output { name, text } => {
            if let Some(agent) = inventory::get_agent(state, &name).await? {
                state
                    .events
                    .append(
                        tower_core::EventKind::AgentOutput,
                        Some("agent"),
                        Some(&agent.id.0),
                        serde_json::json!({ "agent_id": agent.id, "text": text }),
                    )
                    .await?;
            }
            return Ok(());
        }
    };
    if let Some(agent) = inventory::get_agent(state, &name).await? {
        inventory::transition(state, &agent, new_state).await?;
    }
    Ok(())
}
