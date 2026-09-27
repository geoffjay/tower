//! HerdrDriver: drives herdr via its CLI (S1.A findings, plan T4.2).
//!
//! Verb grammar (validated live 2026-09-26/27):
//!   herdr api snapshot                       → inventory
//!   herdr workspace list | create --label L [--cwd D] --no-focus
//!   herdr tab create --workspace <id> --label L [--cwd D] | rename <tab> <L>
//!   herdr agent start <name> --kind <kind> --pane <id> [--timeout <ms>] [-- args]
//!   herdr agent prompt <name> <text> [--wait [--until <s>...] --timeout <ms>]
//!   herdr agent read <name> --source <s> [--lines N] [--format text|ansi]
//!   herdr agent send-keys <name> <key>
//!   herdr agent wait <name> [--until <s>...] [--timeout <ms>]
//!   herdr pane close <pane_id>
//!
//! Output: single-line JSON. Success: {"id":...,"result":{...,"type":...}}.
//! Error: {"error":{"code":...,"message":...},"id":...} with a non-zero exit.
//!
//! Placement: spawned agents live in one herdr workspace labelled
//! `tower-agents` (created on first spawn), one tab per agent rooted at its
//! workdir — never inside the operator's own workspaces.

use std::process::Stdio;

use async_trait::async_trait;
use futures::stream::StreamExt;
use serde::Deserialize;
use tokio::process::Command;

use crate::{
    AgentSpec, DriverError, Harness, HarnessAgent, HarnessEvent, HarnessState, ReadResult,
    ReadSource,
};

#[derive(Debug, Clone, Default)]
pub struct HerdrDriver {
    /// Extra global args before the subcommand (e.g. --remote for phase 5).
    pub herdr_args: Vec<String>,
    /// Poll interval for the event pump.
    pub poll_ms: u64,
}

impl HerdrDriver {
    pub fn new() -> Self {
        Self {
            herdr_args: Vec::new(),
            poll_ms: 1000,
        }
    }

    async fn run_json(&self, args: &[&str]) -> Result<serde_json::Value, DriverError> {
        let out = self
            .herdr_command(args)
            .output()
            .await
            .map_err(|e| DriverError::Transport(format!("spawn herdr: {e}")))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        // CLI prints a leading newline before JSON (observed in S1.A); on
        // failure the error envelope may arrive on either stream.
        let envelope = [stdout.trim(), stderr.trim()]
            .into_iter()
            .find_map(|s| serde_json::from_str::<serde_json::Value>(s).ok());
        if let Some(err) = envelope.as_ref().and_then(|v| v.get("error")) {
            let code = err["code"].as_str().unwrap_or("unknown").to_string();
            let message = err["message"].as_str().unwrap_or("").to_string();
            return Err(map_error(code, message));
        }
        if !out.status.success() {
            return Err(DriverError::Transport(format!(
                "herdr exited {}: {}",
                out.status,
                stderr.trim()
            )));
        }
        let v = envelope.ok_or_else(|| {
            DriverError::Transport(format!("parse herdr output: {:?}", stdout.trim()))
        })?;
        Ok(v["result"].clone())
    }

    /// Run a verb that returns RAW terminal text, not a JSON envelope
    /// (S1.A amendment: `agent read` is the exception — it prints the pane
    /// text directly, framed by a leading newline).
    async fn run_raw(&self, args: &[&str]) -> Result<String, DriverError> {
        let out = self
            .herdr_command(args)
            .output()
            .await
            .map_err(|e| DriverError::Transport(format!("spawn herdr: {e}")))?;
        if !out.status.success() {
            return Err(DriverError::Transport(format!(
                "herdr exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn herdr_command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("herdr");
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for a in &self.herdr_args {
            cmd.arg(a);
        }
        for a in args {
            cmd.arg(a);
        }
        cmd
    }

    /// A fresh pane for `spec`: a new tab (labelled with the agent name,
    /// rooted at its workdir) in the `tower-agents` workspace, creating the
    /// workspace on first use. Serialized so concurrent spawns can't create
    /// two workspaces.
    async fn agent_home(&self, spec: &AgentSpec) -> Result<String, DriverError> {
        static PLACEMENT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _guard = PLACEMENT.lock().await;

        let list = self.run_json(&["workspace", "list"]).await?;
        let existing = list["workspaces"]
            .as_array()
            .and_then(|ws| ws.iter().find(|w| w["label"] == WORKSPACE_LABEL))
            .and_then(|w| w["workspace_id"].as_str().map(str::to_string));

        let mut args: Vec<&str> = Vec::new();
        let created = match &existing {
            Some(ws) => {
                args.extend(["tab", "create", "--workspace", ws, "--label", &spec.name]);
                false
            }
            None => {
                args.extend([
                    "workspace",
                    "create",
                    "--label",
                    WORKSPACE_LABEL,
                    "--no-focus",
                ]);
                true
            }
        };
        if let Some(dir) = &spec.workdir {
            args.extend(["--cwd", dir]);
        }
        let env: Vec<String> = spec.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        for kv in &env {
            args.extend(["--env", kv]);
        }
        let res = self.run_json(&args).await?;
        let pane = res["root_pane"]["pane_id"]
            .as_str()
            .ok_or_else(|| DriverError::Transport("herdr returned no root pane".into()))?
            .to_string();
        if created {
            if let Some(tab) = res["tab"]["tab_id"].as_str() {
                self.run_json(&["tab", "rename", tab, &spec.name]).await?;
            }
        }
        Ok(pane)
    }
}

/// herdr workspace that holds tower-spawned agents.
pub const WORKSPACE_LABEL: &str = "tower-agents";

/// How long `start` waits for a fresh shell to accept `agent start`.
const START_READY_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

fn map_error(code: String, message: String) -> DriverError {
    match code.as_str() {
        "agent_not_found" | "pane_not_found" => DriverError::NotFound(message),
        "agent_prompt_stalled" => DriverError::PromptStalled(message),
        "agent_blocked" => DriverError::AgentBlocked(message),
        "timeout" => DriverError::Timeout(message),
        _ => DriverError::Herdr { code, message },
    }
}

// ---- snapshot types (subset of herdr api schema; unknown fields ignored) ----

#[derive(Debug, Deserialize)]
struct Snapshot {
    #[serde(default)]
    agents: Vec<SnapshotAgent>,
}

#[derive(Debug, Deserialize)]
struct SnapshotAgent {
    /// Absent for panes herdr detected but nobody named (e.g. a harness
    /// launched by hand). Unnamed agents aren't addressable by name, so
    /// tower can neither own nor adopt them; they're skipped.
    #[serde(default)]
    name: Option<String>,
    agent: String,
    pane_id: String,
    agent_status: String,
    #[serde(default)]
    cwd: Option<String>,
}

/// Parse the `result` of `herdr api snapshot` into named harness agents.
pub fn parse_snapshot(result: &serde_json::Value) -> Result<Vec<HarnessAgent>, DriverError> {
    let snap: Snapshot = serde_json::from_value(result["snapshot"].clone())
        .map_err(|e| DriverError::Transport(format!("parse snapshot: {e}")))?;
    Ok(snap
        .agents
        .into_iter()
        .filter_map(|a| {
            Some(HarnessAgent {
                name: a.name?,
                kind: a.agent,
                pane_id: a.pane_id,
                state: HarnessState::from_detection(&a.agent_status)
                    .unwrap_or(HarnessState::Unknown),
                cwd: a.cwd,
            })
        })
        .collect())
}

#[async_trait]
impl Harness for HerdrDriver {
    async fn snapshot(&self) -> Result<Vec<HarnessAgent>, DriverError> {
        parse_snapshot(&self.run_json(&["api", "snapshot"]).await?)
    }

    async fn start(&self, spec: &AgentSpec) -> Result<String, DriverError> {
        let pane_id = self.agent_home(spec).await?;

        let mut args = vec![
            "agent",
            "start",
            spec.name.as_str(),
            "--kind",
            spec.kind.as_str(),
            "--pane",
            pane_id.as_str(),
            "--timeout",
            "60000",
        ];
        if !spec.args.is_empty() {
            args.push("--");
            args.extend(spec.args.iter().map(String::as_str));
        }
        // A fresh tab's shell needs a moment to reach its prompt; herdr
        // answers `agent_pane_busy` until then (S1.A: pane must be at the
        // shell prompt).
        let deadline = std::time::Instant::now() + START_READY_WAIT;
        loop {
            match self.run_json(&args).await {
                Ok(_) => return Ok(pane_id),
                // the harness launched but stopped at a prompt (claude's
                // folder-trust dialog): it exists and is `blocked` — tower
                // surfaces that as an inbox approval, like any other block
                Err(DriverError::Herdr { code, message })
                    if code == "agent_not_ready" && message.contains("blocked") =>
                {
                    return Ok(pane_id)
                }
                Err(DriverError::Herdr { code, .. })
                    if code == "agent_pane_busy" && std::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                Err(e) => {
                    // leave nothing behind in the operator's herdr session
                    let _ = self.run_json(&["pane", "close", &pane_id]).await;
                    return Err(e);
                }
            }
        }
    }

    async fn prompt(&self, name: &str, text: &str, wait: bool) -> Result<(), DriverError> {
        let mut args = vec!["agent", "prompt", name, text];
        if wait {
            args.push("--wait");
            args.push("--timeout");
            args.push("600000"); // 10 min cap; without --timeout herdr waits forever
        }
        self.run_json(&args).await?;
        Ok(())
    }

    async fn interrupt(&self, name: &str) -> Result<(), DriverError> {
        self.run_json(&["agent", "send-keys", name, "ctrl+c"])
            .await?;
        Ok(())
    }

    async fn send_keys(&self, name: &str, keys: &[String]) -> Result<(), DriverError> {
        for key in keys {
            self.run_json(&["agent", "send-keys", name, key]).await?;
        }
        Ok(())
    }

    async fn read(
        &self,
        name: &str,
        source: ReadSource,
        ansi: bool,
    ) -> Result<ReadResult, DriverError> {
        let mut args = vec!["agent", "read", name, "--source", source.as_str()];
        if ansi {
            args.push("--format");
            args.push("ansi");
        }
        // read returns raw text, not JSON (S1.A amendment)
        let text = self.run_raw(&args).await?;
        Ok(ReadResult { text })
    }

    async fn wait_until(
        &self,
        name: &str,
        states: &[HarnessState],
        timeout_ms: u64,
    ) -> Result<(), DriverError> {
        if states.is_empty() {
            return Ok(());
        }
        let mut args = vec!["agent", "wait", name];
        for s in states {
            args.push("--until");
            args.push(s.as_str());
        }
        args.push("--timeout");
        let timeout = timeout_ms.to_string();
        args.push(&timeout);
        self.run_json(&args).await?;
        Ok(())
    }

    async fn stop(&self, name: &str) -> Result<(), DriverError> {
        // find the agent's pane, close it
        let pane = parse_snapshot(&self.run_json(&["api", "snapshot"]).await?)?
            .into_iter()
            .find(|a| a.name == name)
            .map(|a| a.pane_id)
            .ok_or_else(|| DriverError::NotFound(format!("agent {name}")))?;
        self.run_json(&["pane", "close", &pane]).await?;
        Ok(())
    }

    fn events(&self) -> futures::stream::BoxStream<'static, HarnessEvent> {
        let driver = self.clone();
        let poll = std::time::Duration::from_millis(self.poll_ms.max(250));

        // State machine: (driver, last snapshot, per-agent last output)
        struct Pump {
            driver: HerdrDriver,
            prev: Option<Vec<HarnessAgent>>,
            outputs: std::collections::HashMap<String, String>,
        }

        let pump = Pump {
            driver,
            prev: None,
            outputs: std::collections::HashMap::new(),
        };

        let stream = futures::stream::unfold((pump, poll), |(mut pump, poll)| async move {
            loop {
                tokio::time::sleep(poll).await;
                let snap = match pump.driver.snapshot().await {
                    Ok(s) => s,
                    Err(_) => continue, // transient; keep polling
                };

                let mut batch: Vec<HarnessEvent> = Vec::new();

                // up/down + state changes
                for a in &snap {
                    match pump
                        .prev
                        .as_ref()
                        .and_then(|p| p.iter().find(|x| x.name == a.name))
                    {
                        None => batch.push(HarnessEvent::AgentUp(a.clone())),
                        Some(old) if old.state != a.state => {
                            batch.push(HarnessEvent::StateChange {
                                name: a.name.clone(),
                                from: old.state,
                                to: a.state,
                                detail: None,
                            });
                        }
                        _ => {}
                    }
                }
                if let Some(p) = pump.prev.as_ref() {
                    for old in p {
                        if !snap.iter().any(|a| a.name == old.name) {
                            batch.push(HarnessEvent::AgentDown {
                                name: old.name.clone(),
                                pane_id: old.pane_id.clone(),
                            });
                        }
                    }
                }

                // output deltas per agent (recent source, plain text)
                for a in &snap {
                    if let Ok(r) = pump.driver.read(&a.name, ReadSource::Recent, false).await {
                        if r.text.is_empty() {
                            continue;
                        }
                        match pump.outputs.get(&a.name) {
                            Some(prev_text) if *prev_text == r.text => {}
                            Some(prev_text) => {
                                // emit the suffix delta when it grew,
                                // else the whole buffer (screen reshaped)
                                let delta = if r.text.starts_with(prev_text.as_str()) {
                                    r.text[prev_text.len()..].to_string()
                                } else {
                                    r.text.clone()
                                };
                                batch.push(HarnessEvent::Output {
                                    name: a.name.clone(),
                                    text: delta,
                                });
                                pump.outputs.insert(a.name.clone(), r.text);
                            }
                            None => {
                                batch.push(HarnessEvent::Output {
                                    name: a.name.clone(),
                                    text: r.text.clone(),
                                });
                                pump.outputs.insert(a.name.clone(), r.text);
                            }
                        }
                    }
                }

                pump.prev = Some(snap);

                if batch.is_empty() {
                    continue;
                }
                return Some((batch, (pump, poll)));
            }
        })
        .flat_map(futures::stream::iter)
        .boxed();

        stream
    }
}
