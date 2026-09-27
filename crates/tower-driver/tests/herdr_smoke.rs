//! Live herdr smoke test (plan T4.2). Requires a running herdr server;
//! skipped unless `TOWER_E2E=1`. Exercises the real driver verbs end-to-end
//! against a scratch agent.

use tower_driver::{AgentSpec, Harness, ReadSource};

fn herdr_up() -> bool {
    std::env::var("TOWER_E2E").is_ok()
        && std::process::Command::new("herdr")
            .args(["status"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

#[tokio::test]
async fn herdr_snapshot_and_agent_lifecycle() {
    if !herdr_up() {
        eprintln!("skipping: TOWER_E2E not set or herdr down");
        return;
    }

    let driver = tower_driver::herdr::HerdrDriver::new();

    // snapshot works (even with zero agents)
    let snap = driver.snapshot().await.expect("snapshot");
    eprintln!("snapshot: {} agent(s)", snap.len());

    // spawn a pi agent
    let name = format!(
        "tower-smoke-{}",
        tower_core::new_id()
            .to_lowercase()
            .chars()
            .take(8)
            .collect::<String>()
    );
    let spec = AgentSpec {
        name: name.clone(),
        kind: "pi".into(),
        workdir: None,
        args: vec![],
        env: vec![],
    };
    let pane = driver.start(&spec).await.expect("start pi");
    eprintln!("started {name} in pane {pane}");

    // it shows up in the snapshot (herdr's snapshot may lag a beat after
    // start returns — poll briefly)
    let mut found = false;
    for _ in 0..10 {
        let snap = driver.snapshot().await.expect("snapshot after start");
        if snap.iter().any(|a| a.name == name) {
            found = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    assert!(found, "agent must appear in snapshot");

    // read works (pi auth may be unset; the error screen is still output)
    let read = driver.read(&name, ReadSource::Recent, false).await;
    assert!(read.is_ok(), "read must work: {read:?}");

    // prompt delivery (S1.B: no provider auth → agent_prompt_stalled is
    // expected; delivery itself must not be a transport error)
    let prompted = driver.prompt(&name, "say hello", false).await;
    assert!(prompted.is_ok(), "prompt delivery: {prompted:?}");

    // stop: pane closes, agent disappears (allow brief snapshot lag)
    driver.stop(&name, None).await.expect("stop");
    let mut gone = false;
    for _ in 0..10 {
        let snap = driver.snapshot().await.expect("snapshot after stop");
        if !snap.iter().any(|a| a.name == name) {
            gone = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    assert!(gone, "agent must be gone after stop");
    eprintln!("smoke complete for {name}");
}
