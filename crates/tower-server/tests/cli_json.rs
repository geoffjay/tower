//! The `tower` binary's `--json` contract (getting-started: "every verb
//! supports --json"): the herdr mods parse the CLI's stdout as JSON, so a
//! verb that prints prose under `--json` breaks them even when the call
//! succeeded.

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
