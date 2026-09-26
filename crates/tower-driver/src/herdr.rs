//! HerdrDriver: drives herdr via its CLI (S1.A findings, plan T4.2).
//!
//! Verb grammar (validated live 2026-09-26):
//!   herdr api snapshot                       → inventory
//!   herdr pane split --pane <id> --direction right   → new pane
//!   herdr agent start <name> --kind <kind> --pane <id> [--timeout <ms>]
//!   herdr agent prompt <name> <text> [--wait [--until <s>...] --timeout <ms>]
//!   herdr agent read <name> --source <s> [--lines N] [--format text|ansi]
//!   herdr agent send-keys <name> <key>
//!   herdr agent wait <name> [--until <s>...] [--timeout <ms>]
//!   herdr pane close <pane_id>
//!
//! Output: single-line JSON. Success: {"id":...,"result":{...,"type":...}}.
//! Error: {"error":{"code":...,"message":...},"id":...}.

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
        if !out.status.success() {
            return Err(DriverError::Transport(format!(
                "herdr exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        let line = String::from_utf8_lossy(&out.stdout);
        // CLI prints a leading newline before JSON (observed in S1.A)
        let line = line.trim();
        let v: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| DriverError::Transport(format!("parse herdr output: {e}: {line:?}")))?;
        if let Some(err) = v.get("error") {
            let code = err["code"].as_str().unwrap_or("unknown").to_string();
            let message = err["message"].as_str().unwrap_or("").to_string();
            return Err(map_error(code, message));
        }
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
}

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
    #[serde(default)]
    panes: Vec<SnapshotPane>,
}

#[derive(Debug, Deserialize)]
struct SnapshotAgent {
    name: String,
    agent: String,
    pane_id: String,
    agent_status: String,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SnapshotPane {
    pane_id: String,
}

#[async_trait]
impl Harness for HerdrDriver {
    async fn snapshot(&self) -> Result<Vec<HarnessAgent>, DriverError> {
        let result = self.run_json(&["api", "snapshot"]).await?;
        let snap: Snapshot = serde_json::from_value(result["snapshot"].clone())
            .map_err(|e| DriverError::Transport(format!("parse snapshot: {e}")))?;
        Ok(snap
            .agents
            .into_iter()
            .map(|a| HarnessAgent {
                name: a.name,
                kind: a.agent,
                pane_id: a.pane_id,
                state: HarnessState::from_detection(&a.agent_status)
                    .unwrap_or(HarnessState::Unknown),
                cwd: a.cwd,
            })
            .collect())
    }

    async fn start(&self, spec: &AgentSpec) -> Result<String, DriverError> {
        // 1. find a home: use a pane from the first workspace, else split
        let snap_result = self.run_json(&["api", "snapshot"]).await?;
        let snap: Snapshot = serde_json::from_value(snap_result["snapshot"].clone())
            .map_err(|e| DriverError::Transport(format!("parse snapshot: {e}")))?;

        let pane_id = if let Some(pane) = snap.panes.first() {
            // split an existing pane to get a fresh shell
            let split = self
                .run_json(&[
                    "pane",
                    "split",
                    "--pane",
                    &pane.pane_id,
                    "--direction",
                    "right",
                ])
                .await?;
            split["pane"]["pane_id"]
                .as_str()
                .ok_or_else(|| DriverError::Transport("split returned no pane id".into()))?
                .to_string()
        } else {
            return Err(DriverError::Transport("no panes to split from".into()));
        };

        // 2. start the agent in the new pane (detection-wait ≤ 60s)
        let result = self
            .run_json(&[
                "agent",
                "start",
                &spec.name,
                "--kind",
                &spec.kind,
                "--pane",
                &pane_id,
                "--timeout",
                "60000",
            ])
            .await?;

        let _ = result; // agent_started envelope; pane_id is what we need
        Ok(pane_id)
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
        let snap_result = self.run_json(&["api", "snapshot"]).await?;
        let snap: Snapshot = serde_json::from_value(snap_result["snapshot"].clone())
            .map_err(|e| DriverError::Transport(format!("parse snapshot: {e}")))?;
        let pane = snap
            .agents
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.pane_id.clone())
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
