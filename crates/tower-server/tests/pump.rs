//! Pump integration tests (plan T2.1 verify): blocked detection → exactly
//! one auto-question to the operator, deduped across repeated blocked
//! events in the same episode.

use std::sync::Arc;
use std::time::Duration;

use tower_driver::fake::FakeHarness;
use tower_driver::{HarnessEvent, HarnessState};
use tower_server::{AppState, Config, Paths};

async fn boot(_name: &str) -> (AppState, FakeHarness, tempdir::TempDir) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = tempdir::TempDir::new(&format!("tower-pump-{n}")).unwrap();
    let paths = Paths {
        config_file: dir.path().join("config.toml"),
        db_file: dir.path().join("tower.db"),
        artifacts_dir: dir.path().join("artifacts"),
        token_file: dir.path().join("token"),
        socket_file: dir.path().join("tower.sock"),
    };
    paths.ensure_dirs().unwrap();
    let pool = tower_server::open_db(&paths.db_file).await.unwrap();
    let events = tower_server::EventLog::attach(&pool).await.unwrap();
    let harness = FakeHarness::new();
    let state = AppState::new(
        pool,
        events,
        Config::default(),
        "t".into(),
        Arc::new(harness.clone()),
    );
    tower_server::inventory::ensure_local_machine(&state)
        .await
        .unwrap();
    let _ = n;
    (state, harness, dir)
}

async fn pending_questions(state: &AppState) -> i64 {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages WHERE kind='question' AND status='pending'",
    )
    .fetch_one(&state.pool)
    .await
    .unwrap();
    count
}

#[tokio::test]
async fn blocked_detection_creates_exactly_one_question() {
    let (state, harness, _d) = boot("blocked").await;
    // spawn an agent row first (pump only tracks owned agents)
    let agent = tower_server::sessions::spawn(
        &state,
        tower_server::sessions::SpawnRequest {
            name: "backend".into(),
            kind: Some("pi".into()),
            workdir: Some("/tmp".into()),
            worktree: false,
            adopt: false,
            prompt: None,
            permissions: None,
        },
    )
    .await
    .unwrap();
    let _ = agent;

    // queue the blocked transition, then start the pump
    harness.push_event(HarnessEvent::StateChange {
        name: "backend".into(),
        from: HarnessState::Working,
        to: HarnessState::Blocked,
        detail: None,
    });
    // same-episode repeat: must NOT create a second question
    harness.push_event(HarnessEvent::StateChange {
        name: "backend".into(),
        from: HarnessState::Blocked,
        to: HarnessState::Blocked,
        detail: None,
    });
    let _pump = tower_server::pump::spawn(state.clone());

    // pump drains the scripted queue quickly; poll until question appears
    for _ in 0..50 {
        if pending_questions(&state).await > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // settle: give the second event time to (wrongly) create another
    tokio::time::sleep(Duration::from_millis(100)).await;

    let count = pending_questions(&state).await;
    assert_eq!(count, 1, "one question per blocked episode, got {count}");
}

#[tokio::test]
async fn blocked_claude_agent_opens_an_approval() {
    let (state, harness, _d) = boot("claude").await;
    tower_server::sessions::spawn(
        &state,
        tower_server::sessions::SpawnRequest {
            name: "coder".into(),
            kind: Some("claude".into()),
            workdir: Some("/tmp".into()),
            worktree: false,
            adopt: false,
            prompt: None,
            permissions: None,
        },
    )
    .await
    .unwrap();
    harness.push_event(HarnessEvent::StateChange {
        name: "coder".into(),
        from: HarnessState::Working,
        to: HarnessState::Blocked,
        detail: None,
    });
    let _pump = tower_server::pump::spawn(state.clone());

    let mut kind: Option<String> = None;
    for _ in 0..50 {
        kind = sqlx::query_scalar("SELECT kind FROM messages WHERE from_id='coder'")
            .fetch_optional(&state.pool)
            .await
            .unwrap();
        if kind.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(kind.as_deref(), Some("approval"));
}
