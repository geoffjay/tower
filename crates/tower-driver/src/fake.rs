//! FakeHarness: scripted driver for integration tests (D§16, plan T5.1).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};

use crate::{
    AgentSpec, DriverError, Harness, HarnessAgent, HarnessEvent, HarnessState, ReadResult,
    ReadSource,
};

/// A scriptable in-memory harness.
#[derive(Clone, Default)]
pub struct FakeHarness {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    agents: Vec<FakeAgent>,
    /// Recorded prompt calls: (name, text, waited)
    prompts: Vec<(String, String, bool)>,
    next_pane: usize,
}

struct FakeAgent {
    name: String,
    kind: String,
    pane_id: String,
    state: HarnessState,
    output: String,
    up: bool,
}

impl FakeHarness {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a pre-existing agent (adoption scenario).
    pub fn with_agent(self, name: &str, kind: &str, state: HarnessState) -> Self {
        {
            let mut i = self.inner.lock().unwrap();
            let pane_id = format!("fake:p{}", i.next_pane);
            i.next_pane += 1;
            let name = name.to_string();
            let kind = kind.to_string();
            i.agents.push(FakeAgent {
                name,
                kind,
                pane_id,
                state,
                output: String::new(),
                up: true,
            });
        }
        self
    }

    /// Transition an agent's state (drives reconcile events in tests).
    pub fn set_state(&self, name: &str, state: HarnessState) {
        let mut i = self.inner.lock().unwrap();
        if let Some(a) = i.agents.iter_mut().find(|a| a.name == name && a.up) {
            a.state = state;
        }
    }

    pub fn append_output(&self, name: &str, text: &str) {
        let mut i = self.inner.lock().unwrap();
        if let Some(a) = i.agents.iter_mut().find(|a| a.name == name && a.up) {
            a.output.push_str(text);
        }
    }

    pub fn kill(&self, name: &str) {
        let mut i = self.inner.lock().unwrap();
        if let Some(a) = i.agents.iter_mut().find(|a| a.name == name) {
            a.up = false;
        }
    }

    pub fn prompts(&self) -> Vec<(String, String, bool)> {
        self.inner.lock().unwrap().prompts.clone()
    }
}

#[async_trait]
impl Harness for FakeHarness {
    async fn snapshot(&self) -> Result<Vec<HarnessAgent>, DriverError> {
        let i = self.inner.lock().unwrap();
        Ok(i.agents
            .iter()
            .filter(|a| a.up)
            .map(|a| HarnessAgent {
                name: a.name.clone(),
                kind: a.kind.clone(),
                pane_id: a.pane_id.clone(),
                state: a.state,
                cwd: None,
            })
            .collect())
    }

    async fn start(&self, spec: &AgentSpec) -> Result<String, DriverError> {
        let mut i = self.inner.lock().unwrap();
        let pane = format!("fake:p{}", i.next_pane);
        i.next_pane += 1;
        let name = spec.name.clone();
        let kind = spec.kind.clone();
        i.agents.push(FakeAgent {
            name,
            kind,
            pane_id: pane.clone(),
            state: HarnessState::Idle,
            output: String::new(),
            up: true,
        });
        Ok(pane)
    }

    async fn prompt(&self, name: &str, text: &str, wait: bool) -> Result<(), DriverError> {
        let mut i = self.inner.lock().unwrap();
        let a = i
            .agents
            .iter_mut()
            .find(|a| a.name == name && a.up)
            .ok_or_else(|| DriverError::NotFound(name.into()))?;
        a.state = HarnessState::Working;
        i.prompts.push((name.into(), text.into(), wait));
        Ok(())
    }

    async fn interrupt(&self, _name: &str) -> Result<(), DriverError> {
        Ok(())
    }

    async fn read(
        &self,
        name: &str,
        _source: ReadSource,
        _ansi: bool,
    ) -> Result<ReadResult, DriverError> {
        let i = self.inner.lock().unwrap();
        let a = i
            .agents
            .iter()
            .find(|a| a.name == name && a.up)
            .ok_or_else(|| DriverError::NotFound(name.into()))?;
        Ok(ReadResult {
            text: a.output.clone(),
        })
    }

    async fn wait_until(
        &self,
        name: &str,
        states: &[HarnessState],
        _timeout_ms: u64,
    ) -> Result<(), DriverError> {
        let i = self.inner.lock().unwrap();
        match i.agents.iter().find(|a| a.name == name && a.up) {
            Some(a) if states.contains(&a.state) => Ok(()),
            Some(_) => Err(DriverError::Timeout(format!("waiting for {name}"))),
            None => Err(DriverError::NotFound(name.into())),
        }
    }

    async fn stop(&self, name: &str) -> Result<(), DriverError> {
        self.kill(name);
        Ok(())
    }

    fn events(&self) -> BoxStream<'static, HarnessEvent> {
        futures::stream::pending().boxed()
    }
}
