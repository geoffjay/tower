//! Server integration tests: inventory reconcile + control routes with
//! FakeHarness (plan T5.1/T5.2 verify, D§16).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower_driver::fake::FakeHarness;
use tower_driver::HarnessState;
use tower_server::AppState;
use tower_server::{Config, Paths};

fn temp_paths(n: &str) -> (Paths, tempfile::TempDir) {
    let dir = tempfile::Builder::new().prefix(n).tempdir().unwrap();
    let paths = Paths {
        config_file: dir.path().join("config.toml"),
        db_file: dir.path().join("tower.db"),
        artifacts_dir: dir.path().join("artifacts"),
        token_file: dir.path().join("token"),
        socket_file: dir.path().join("tower.sock"),
        agents_dir: dir.path().join("agents"),
    };
    paths.ensure_dirs().unwrap();
    (paths, dir)
}

async fn boot(harness: FakeHarness) -> (AppState, FakeHarness, tempfile::TempDir) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let (paths, dir) = temp_paths(&format!("tower-m5-{n}"));
    let pool = tower_server::open_db(&paths.db_file).await.unwrap();
    let events = tower_server::EventLog::attach(&pool).await.unwrap();
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
    (state, harness, dir)
}

fn router(state: AppState) -> axum::Router {
    tower_server::serve::router(state)
}

async fn json_req(
    router: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let req = match body {
        Some(v) => Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
    };
    let resp = tower::ServiceExt::oneshot(router.clone(), req)
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or_default()
    };
    (status, v)
}

#[tokio::test]
async fn reconcile_marks_dead_and_finds_adoptable() {
    let (state, harness, _d) =
        boot(FakeHarness::new().with_agent("stray", "pi", HarnessState::Idle)).await;

    // reconcile sees the stray as adoptable
    let candidates = tower_server::inventory::adoption_candidates(&state)
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0]["name"], "stray");

    // adopt it
    let adopted = tower_server::inventory::adopt(&state, "stray")
        .await
        .unwrap();
    assert!(adopted.adopted);
    assert_eq!(adopted.state, tower_core::AgentState::Idle);

    // it's no longer adoptable
    let candidates = tower_server::inventory::adoption_candidates(&state)
        .await
        .unwrap();
    assert_eq!(candidates.len(), 0);

    // harness agent disappears → reconcile marks the row dead
    harness.kill("stray");
    tower_server::inventory::reconcile(&state).await.unwrap();
    let agent = tower_server::inventory::get_agent(&state, "stray")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(agent.state, tower_core::AgentState::Dead);
}

#[tokio::test]
async fn spawn_prompt_stop_routes() {
    let (state, harness, _d) = boot(FakeHarness::new()).await;
    let app = router(state.clone());

    // spawn
    let (status, v) = json_req(
        &app,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({"name": "writer", "kind": "pi"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["agent"]["name"], "writer");
    assert_eq!(v["agent"]["kind"], "pi");

    // duplicate spawn → conflict
    let (status, _) = json_req(
        &app,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({"name": "writer", "kind": "pi"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // list
    let (status, v) = json_req(&app, "GET", "/v1/agents", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["agents"].as_array().unwrap().len(), 1);

    // prompt
    let (status, v) = json_req(
        &app,
        "POST",
        "/v1/agents/writer/prompt",
        Some(serde_json::json!({"text": "do the thing", "wait": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], true);

    // the harness saw the prompt
    let prompts = harness.prompts();
    assert_eq!(
        prompts,
        vec![("writer".to_string(), "do the thing".into(), false)]
    );

    // show
    let (status, v) = json_req(&app, "GET", "/v1/agents/writer", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["agent"]["name"], "writer");
    // unset columns are null, not "" (clients render "worktree " otherwise)
    assert!(v["agent"]["worktree"].is_null(), "{v}");
    assert!(v["agent"]["workdir"].is_null(), "{v}");
    // prompt flipped the fake's state to Working
    assert_eq!(v["agent"]["state"], "working");

    // read
    let (status, v) = json_req(&app, "GET", "/v1/agents/writer/read", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(v["output"].is_string());

    // unknown agent → 404 envelope
    let (status, v) = json_req(&app, "GET", "/v1/agents/ghost", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(v["error"]["code"], "not_found");

    // stop (keep row)
    let (status, _) = json_req(
        &app,
        "POST",
        "/v1/agents/writer/stop",
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let agent = tower_server::inventory::get_agent(&state, "writer")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(agent.state, tower_core::AgentState::Dead);
}

#[tokio::test]
async fn adopt_via_spawn_route() {
    let (state, _harness, _d) =
        boot(FakeHarness::new().with_agent("found", "claude", HarnessState::Working)).await;
    let app = router(state);

    let (status, v) = json_req(
        &app,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({"name": "found", "adopt": true})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["agent"]["adopted"], true);
    assert_eq!(v["agent"]["state"], "working");
}

#[tokio::test]
async fn spawn_passes_model_and_args_to_harness() {
    let (state, harness, _d) = boot(FakeHarness::new()).await;
    let app = router(state.clone());

    // model + per-agent args: harness sees kind args? none configured →
    // args then --model
    let (status, v) = json_req(
        &app,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({
            "name": "m1", "kind": "omp",
            "model": "anthropic/claude-sonnet-5-5",
            "args": ["--smol", "--plan"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(
        v["agent"]["model"], "anthropic/claude-sonnet-5-5",
        "model is stored on the row"
    );
    assert_eq!(v["agent"]["args"], serde_json::json!(["--smol", "--plan"]));
    assert_eq!(
        harness.spawns(),
        vec![(
            "m1".into(),
            vec![
                "--smol".into(),
                "--plan".into(),
                "--model".into(),
                "anthropic/claude-sonnet-5-5".into()
            ]
        )],
        "harness received per-agent args then --model"
    );

    // no model, no args → none passed
    let (status, _v) = json_req(
        &app,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({"name": "m2", "kind": "pi"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        harness.spawns(),
        vec![
            (
                "m1".into(),
                vec![
                    "--smol".into(),
                    "--plan".into(),
                    "--model".into(),
                    "anthropic/claude-sonnet-5-5".into()
                ]
            ),
            ("m2".into(), vec![]),
        ]
    );

    // ps json shows the model
    let (status, v) = json_req(&app, "GET", "/v1/agents", None).await;
    assert_eq!(status, StatusCode::OK);
    let m1 = v["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "m1")
        .unwrap();
    assert_eq!(m1["model"], "anthropic/claude-sonnet-5-5");
}

#[tokio::test]
async fn spawn_applies_kind_args_from_config() {
    // boot() uses Config::default(); build a configured state by hand
    let dir = tempfile::Builder::new()
        .prefix("tower-m5-kindargs")
        .tempdir()
        .unwrap();
    let paths = Paths {
        config_file: dir.path().join("config.toml"),
        db_file: dir.path().join("tower.db"),
        artifacts_dir: dir.path().join("artifacts"),
        token_file: dir.path().join("token"),
        socket_file: dir.path().join("tower.sock"),
        agents_dir: dir.path().join("agents"),
    };
    paths.ensure_dirs().unwrap();
    let cfg: Config = toml::from_str("[harness.omp]\nargs = [\"--compact\"]\n").unwrap();
    let pool = tower_server::open_db(&paths.db_file).await.unwrap();
    let events = tower_server::EventLog::attach(&pool).await.unwrap();
    let harness = FakeHarness::new();
    let state = AppState::new(
        pool,
        events,
        cfg,
        "t".into(),
        std::sync::Arc::new(harness.clone()),
    );
    tower_server::inventory::ensure_local_machine(&state)
        .await
        .unwrap();
    let app = router(state);

    // kind args first, then per-agent args, then --model
    let (status, v) = json_req(
        &app,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({
            "name": "k1", "kind": "omp",
            "model": "gpt-6", "args": ["--smol"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(
        harness.spawns(),
        vec![(
            "k1".into(),
            vec![
                "--compact".into(), // [harness.omp] args
                "--smol".into(),    // per-agent args
                "--model".into(),
                "gpt-6".into(),
            ]
        )]
    );
    // the row stores only per-agent args, not the kind defaults
    assert_eq!(v["agent"]["args"], serde_json::json!(["--smol"]));
}

#[tokio::test]
async fn opencode_launches_standalone_with_the_model_in_its_env() {
    let (state, harness, _d) = boot(FakeHarness::new()).await;
    let app = router(state.clone());

    // the TUI rejects `--model`: the model rides in OPENCODE_CONFIG_CONTENT,
    // and `--standalone` keeps the pane env (TOWER_AGENT, model) in reach
    let (status, v) = json_req(
        &app,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({
            "name": "oc", "kind": "opencode",
            "model": "opencode/big-pickle", "args": ["--auto"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["agent"]["model"], "opencode/big-pickle");
    let spec = harness.specs().pop().unwrap();
    assert_eq!(spec.args, ["--standalone", "--auto"]);
    let config = spec
        .env
        .iter()
        .find(|(k, _)| k == "OPENCODE_CONFIG_CONTENT")
        .map(|(_, v)| serde_json::from_str::<serde_json::Value>(v).unwrap());
    assert_eq!(
        config,
        Some(serde_json::json!({"model": "opencode/big-pickle"}))
    );
    assert!(spec
        .env
        .iter()
        .any(|(k, v)| k == "TOWER_AGENT" && v == "oc"));

    // resume keeps `--standalone`; opencode takes `--session <id>`
    harness.kill("oc");
    tower_server::inventory::reconcile(&state).await.unwrap();
    let (status, v) = json_req(
        &app,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({"name": "oc", "resume": "ses_42"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(
        harness.specs().pop().unwrap().args,
        ["--standalone", "--session", "ses_42"]
    );
}

#[tokio::test]
async fn readopt_dead_row_keeps_its_id_and_renames_the_harness() {
    let (state, harness, _d) =
        boot(FakeHarness::new().with_agent("qa", "omp", HarnessState::Idle)).await;
    // tower owns qa, then its harness dies in its pane
    let first = tower_server::inventory::adopt(&state, "qa").await.unwrap();
    harness.kill("qa");
    tower_server::inventory::reconcile(&state).await.unwrap();
    let row = tower_server::inventory::get_agent(&state, "qa")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, tower_core::AgentState::Dead);
    let pane = row.pane_id.clone().unwrap();

    // a harness restarted by hand in the same pane: unnamed
    harness.restart_by_hand(&pane, "omp", HarnessState::Working);

    // re-adopt: same id, state follows the harness, herdr name restored
    let again = tower_server::inventory::adopt(&state, "qa").await.unwrap();
    assert_eq!(again.id, first.id, "re-adopt must keep the row id");
    assert_eq!(again.state, tower_core::AgentState::Working);
    assert_eq!(harness.renames(), vec![(pane.clone(), "qa".into())]);

    // a row that is not dead still refuses
    let err = tower_server::inventory::adopt(&state, "qa")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("only dead rows"), "{err}");
}

#[tokio::test]
async fn resume_relaunches_a_dead_row_with_per_kind_args() {
    let (state, harness, _d) =
        boot(FakeHarness::new().with_agent("s1", "omp", HarnessState::Idle)).await;
    tower_server::inventory::adopt(&state, "s1").await.unwrap();
    harness.kill("s1");
    tower_server::inventory::reconcile(&state).await.unwrap();
    let row = tower_server::inventory::get_agent(&state, "s1")
        .await
        .unwrap()
        .unwrap();
    let pane = row.pane_id.clone().unwrap();
    harness.set_session(&pane, "/sessions/old.jsonl");

    // resume the reported session: omp takes `--resume <session>`
    let app = router(state.clone());
    let (status, v) = json_req(
        &app,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({"name": "s1", "resume": ""})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["agent"]["id"], row.id.0, "resume must keep the row id");
    assert_eq!(
        harness.spawns(),
        vec![(
            "s1".into(),
            vec!["--resume".into(), "/sessions/old.jsonl".into()]
        )]
    );

    // explicit session for pi: `--session <id>`
    let (state2, harness2, _d2) =
        boot(FakeHarness::new().with_agent("p1", "pi", HarnessState::Idle)).await;
    tower_server::inventory::adopt(&state2, "p1").await.unwrap();
    harness2.kill("p1");
    tower_server::inventory::reconcile(&state2).await.unwrap();
    let row2 = tower_server::inventory::get_agent(&state2, "p1")
        .await
        .unwrap()
        .unwrap();
    let app2 = router(state2.clone());
    let (status, v) = json_req(
        &app2,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({"name": "p1", "resume": "sess-9"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["agent"]["id"], row2.id.0, "resume must keep the row id");
    assert_eq!(
        harness2.spawns(),
        vec![("p1".into(), vec!["--session".into(), "sess-9".into()])]
    );

    // --continue on a dead row maps to the harness's own continue flag;
    // p1 (resumed above, now live) refuses instead
    let (status, v) = json_req(
        &app2,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({"name": "p1", "continue": true})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");

    let (state3, harness3, _d3) =
        boot(FakeHarness::new().with_agent("c1", "omp", HarnessState::Idle)).await;
    tower_server::inventory::adopt(&state3, "c1").await.unwrap();
    harness3.kill("c1");
    tower_server::inventory::reconcile(&state3).await.unwrap();
    let app3 = router(state3.clone());
    let (status, v) = json_req(
        &app3,
        "POST",
        "/v1/agents",
        Some(serde_json::json!({"name": "c1", "continue": true})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(
        harness3.spawns(),
        vec![("c1".into(), vec!["--continue".into()])]
    );
}

#[tokio::test]
async fn resume_and_adopt_refuse_a_row_that_is_not_dead() {
    let (state, _harness, _d) =
        boot(FakeHarness::new().with_agent("live", "pi", HarnessState::Idle)).await;
    tower_server::inventory::adopt(&state, "live")
        .await
        .unwrap();
    let app = router(state.clone());
    for body in [
        serde_json::json!({"name": "live", "resume": "s"}),
        serde_json::json!({"name": "live", "continue": true}),
        serde_json::json!({"name": "live", "adopt": true}),
    ] {
        let (status, v) = json_req(&app, "POST", "/v1/agents", Some(body)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
    }
}

/// A git checkout in a temp dir with one commit (identity via -c: CI
/// runners have no git user configured).
fn git_repo() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("tower-wt")
        .tempdir()
        .unwrap();
    for args in [
        &["init", "-q"][..],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ][..],
    ] {
        git(dir.path(), args);
    }
    dir
}

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Issue #2: the harness runs in the worktree, a re-created agent reuses it,
/// and `stop --remove` removes it only when clean.
#[tokio::test]
async fn worktree_spawn_runs_the_harness_in_the_worktree_and_reuses_it() {
    let (state, harness, _d) = boot(FakeHarness::new()).await;
    let app = router(state);
    let repo = git_repo();
    let workdir = repo.path().to_str().unwrap().to_string();
    let expected = format!("{workdir}/.worktrees/wt");
    let spawn = serde_json::json!({
        "name": "wt", "kind": "pi", "workdir": workdir, "worktree": true,
    });

    let (status, v) = json_req(&app, "POST", "/v1/agents", Some(spawn.clone())).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    // the harness starts in the worktree, and the row reports the same path
    assert_eq!(
        harness.specs()[0].workdir.as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(v["agent"]["worktree"], expected);
    assert_eq!(v["agent"]["workdir"], workdir);
    let wt = std::path::Path::new(&expected);
    assert_eq!(
        git(wt, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "HEAD",
        "detached HEAD"
    );

    // a dirty worktree survives `stop --remove`, with git's reason
    std::fs::write(wt.join("notes.txt"), "work in progress").unwrap();
    let (status, v) = json_req(
        &app,
        "POST",
        "/v1/agents/wt/stop",
        Some(serde_json::json!({"remove": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["worktree"]["removed"], false, "{v}");
    assert!(
        v["worktree"]["reason"]
            .as_str()
            .unwrap()
            .contains("untracked"),
        "{v}"
    );
    assert!(wt.join("notes.txt").exists());

    // re-creating the agent reuses its worktree and its files
    let (status, v) = json_req(&app, "POST", "/v1/agents", Some(spawn)).await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(
        harness.specs()[1].workdir.as_deref(),
        Some(expected.as_str())
    );
    assert!(wt.join("notes.txt").exists());

    // once clean, `stop --remove` removes it from disk and from git
    std::fs::remove_file(wt.join("notes.txt")).unwrap();
    let (status, v) = json_req(
        &app,
        "POST",
        "/v1/agents/wt/stop",
        Some(serde_json::json!({"remove": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["worktree"]["removed"], true, "{v}");
    assert!(!wt.exists());
    assert!(!git(repo.path(), &["worktree", "list"]).contains(".worktrees/wt"));
}

#[tokio::test]
async fn worktree_that_cannot_be_created_fails_the_spawn() {
    let (state, harness, _d) = boot(FakeHarness::new()).await;
    let app = router(state.clone());
    let not_git = tempfile::Builder::new()
        .prefix("tower-nogit")
        .tempdir()
        .unwrap();
    let repo = git_repo();
    for (body, why) in [
        (
            serde_json::json!({"name": "x", "workdir": not_git.path(), "worktree": true}),
            "not a git repository",
        ),
        (
            serde_json::json!({"name": "x", "worktree": true}),
            "needs a workdir",
        ),
        (
            serde_json::json!({"name": "../x", "workdir": repo.path(), "worktree": true}),
            "can't name a worktree directory",
        ),
    ] {
        let (status, v) = json_req(&app, "POST", "/v1/agents", Some(body)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert!(v["error"]["message"].as_str().unwrap().contains(why), "{v}");
    }
    // no fallback to the main checkout: nothing started, no row
    assert!(harness.specs().is_empty());
    assert!(tower_server::inventory::get_agent(&state, "x")
        .await
        .unwrap()
        .is_none());
}
