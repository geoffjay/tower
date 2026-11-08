//! The `tower` binary as scripts and the herdr mods run it: every verb's
//! `--json` (getting-started: "every verb supports --json") - the mods
//! parse the CLI's stdout as JSON, so a verb that prints prose under
//! `--json` breaks them even when the call succeeded - and no question
//! asked without a terminal.

mod common;

use std::net::SocketAddr;

/// Run the built `tower` binary against the server at `addr`; stdout as JSON.
async fn tower_json(addr: SocketAddr, args: &[&str]) -> serde_json::Value {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_tower"));
    cmd.args(args)
        .arg("--json")
        .env("TOWER_URL", format!("http://{addr}/"))
        .env("TOWER_TOKEN", "t");
    let out = tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{args:?}: {stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{args:?} --json: {e}: {stdout:?}"))
}

#[tokio::test]
async fn write_verbs_print_the_api_json() {
    let ctx = common::boot().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = ctx.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    ctx.spawn("writer", "pi").await;
    let v = tower_json(addr, &["prompt", "writer", "summarize the repo"]).await;
    assert!(v["state"].is_string(), "{v}");
    assert_eq!(
        ctx.harness.prompts(),
        vec![("writer".into(), "summarize the repo".into(), false)]
    );

    let id = ctx.ask_operator("writer", "question", 300).await;
    let v = tower_json(addr, &["approve", &id, "--answer", "yes"]).await;
    assert_eq!(v["message"]["id"], id.as_str(), "{v}");
    assert_eq!(ctx.status_of(&id).await, "answered");
}

/// `task create` / `workflow run` ask for a missing positional only in a
/// terminal (D§10): from a script or an agent's shell they must fail at
/// once, not wait on a question nobody can answer.
#[tokio::test]
async fn a_missing_positional_without_a_terminal_fails_instead_of_asking() {
    let ctx = common::boot().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = ctx.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    for (args, what) in [
        (["task", "create"], "the job title"),
        (["workflow", "run"], "the workflow name"),
    ] {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_tower"));
        cmd.args(args)
            .stdin(std::process::Stdio::null())
            .env("TOWER_URL", format!("http://{addr}/"))
            .env("TOWER_TOKEN", "t");
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio::task::spawn_blocking(move || cmd.output().unwrap()),
        )
        .await
        .expect("no terminal: fails, never waits")
        .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{args:?} succeeded");
        assert!(
            stderr.contains(&format!("missing {what}")) && stderr.contains("terminal"),
            "{args:?}: {stderr}"
        );
    }
    let (_, v) = ctx.req("GET", "/v1/tasks", None).await;
    assert_eq!(v["tasks"], serde_json::json!([]), "nothing created: {v}");
}
