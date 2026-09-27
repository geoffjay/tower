//! TUI against a real server (plan T3.1 / T2.2 / T5.2 verify): the full
//! router on a TCP port with a scripted FakeHarness; the TUI's own feed,
//! reducers, key handling and `/v1` calls — only the terminal is headless.

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tower_client::Client;
use tower_tui::app::{App, Conn, Effect, ViewId};
use tower_tui::feed::{self, FeedMsg};
use tower_tui::fmt::Tz;

/// Serve the router on `listener`; aborting the task drops every open
/// connection too (a crash, as far as clients can tell).
fn serve(listener: tokio::net::TcpListener, app: axum::Router) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut conns = tokio::task::JoinSet::new();
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let app = app.clone();
            conns.spawn(async move {
                let svc =
                    hyper::service::service_fn(move |req: http::Request<hyper::body::Incoming>| {
                        let mut app = app.clone();
                        async move { tower_service::Service::call(&mut app, req).await }
                    });
                let _ = hyper_util::server::conn::auto::Builder::new(
                    hyper_util::rt::TokioExecutor::new(),
                )
                .serve_connection(hyper_util::rt::TokioIo::new(stream), svc)
                .await;
            });
        }
    })
}

struct Live {
    ctx: common::Ctx,
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    client: Client,
    app: App,
    feed_rx: UnboundedReceiver<FeedMsg>,
}

async fn live() -> Live {
    let ctx = common::boot().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = serve(listener, ctx.router.clone());
    let client = Client::with_base(format!("http://{addr}/").parse().unwrap(), "t");
    let (tx, feed_rx) = unbounded_channel();
    tokio::spawn(feed::run(client.clone(), tx));
    let mut l = Live {
        ctx,
        addr,
        server,
        client,
        app: App::new(tower_core::now_ms(), Tz::utc()),
        feed_rx,
    };
    l.until(|a| a.conn == Conn::Live).await;
    l
}

impl Live {
    /// Execute effects the way the runtime does, feeding results back.
    async fn settle(&mut self, mut fx: Vec<Effect>) {
        while let Some(e) = fx.pop() {
            match e {
                Effect::Fetch(f) => {
                    let r = tower_tui::exec::fetch(&self.client, &f)
                        .await
                        .map_err(|e| format!("{e:#}"));
                    fx.extend(self.app.on_loaded(f, r));
                }
                Effect::Api(a) => {
                    let r = tower_tui::exec::api(&self.client, &a)
                        .await
                        .map_err(|e| format!("{e:#}"));
                    self.app.on_api(r);
                }
                Effect::Attach { .. } => panic!("no herdr in tests"),
            }
        }
    }

    /// Apply feed messages until `done` holds (5s cap).
    async fn until(&mut self, done: impl Fn(&App) -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !done(&self.app) {
            let m = tokio::time::timeout_at(deadline, self.feed_rx.recv())
                .await
                .expect("condition not reached within 5s")
                .expect("feed ended");
            let fx = self.app.on_feed(m);
            self.settle(fx).await;
        }
    }

    async fn key(&mut self, code: KeyCode) {
        let fx = self.app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        self.settle(fx).await;
    }

    async fn typed(&mut self, s: &str) {
        for c in s.chars() {
            self.key(KeyCode::Char(c)).await;
        }
    }

    fn frame(&mut self) -> String {
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
        t.draw(|f| self.app.render(f)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// claude's tool-permission menu; approve must pick the plain "Yes".
const PERMISSION_MENU: &str = " Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and don't ask again for echo commands in /tmp
   3. No, and tell Claude what to do differently (esc)";

#[tokio::test]
async fn approve_from_the_inbox() {
    let mut l = live().await;
    l.ctx.spawn("backend", "claude").await;
    l.ctx.harness.append_output("backend", PERMISSION_MENU);
    let id = l.ctx.ask_operator("backend", "approval", 300).await;

    l.until(|a| a.store.inbox.len() == 1).await;
    l.key(KeyCode::Char('2')).await;
    assert_eq!(l.app.view, ViewId::Inbox);
    let frame = l.frame();
    assert!(
        frame.contains("approval") && frame.contains("backend"),
        "{frame}"
    );
    // the fleet row says why it waits
    assert!(l.app.store.agents.iter().any(|a| a.name == "backend"));

    l.key(KeyCode::Char('y')).await;
    let toast = l.app.toast.clone().expect("result toast");
    assert!(!toast.error, "{}", toast.text);
    assert_eq!(toast.text, "approved → delivered to backend");
    assert_eq!(
        l.ctx.harness.keys(),
        vec![("backend".into(), vec!["enter".into()])]
    );
    assert_eq!(l.ctx.status_of(&id).await, "answered");

    // message.status on the bus empties the inbox without a manual refresh
    l.until(|a| a.store.inbox.is_empty()).await;
    assert!(l.frame().contains("inbox empty"));
}

#[tokio::test]
async fn prompt_input_posts_to_the_agent() {
    let mut l = live().await;
    l.ctx.spawn("writer", "pi").await;
    l.until(|a| a.store.agents.iter().any(|x| x.name == "writer"))
        .await;
    l.key(KeyCode::Enter).await; // open detail (reads output + history)
    assert_eq!(l.app.view, ViewId::Agent);
    l.key(KeyCode::Char('i')).await;
    l.typed("summarize the repo").await;
    l.key(KeyCode::Enter).await;
    assert_eq!(
        l.ctx.harness.prompts(),
        vec![("writer".into(), "summarize the repo".into(), false)]
    );
    assert_eq!(
        l.app.toast.as_ref().map(|t| t.text.as_str()),
        Some("prompt delivered → writer")
    );
    // the prompt is a message row: the open history picks it up off the bus
    l.until(|a| {
        a.store.history.as_ref().is_some_and(|(_, m)| {
            m.iter()
                .any(|m| m.parts[0].text.as_deref() == Some("summarize the repo"))
        })
    })
    .await;
    assert!(l.frame().contains("me → writer prompt"));
}

#[tokio::test]
async fn feed_resumes_from_its_cursor_across_a_server_restart() {
    let mut l = live().await;
    let (status, _) = l
        .ctx
        .req(
            "POST",
            "/v1/tasks",
            Some(serde_json::json!({"title": "before"})),
        )
        .await;
    assert!(status.is_success());
    l.until(|a| a.store.tasks.len() == 1).await;

    // crash the HTTP side; the feed notices and starts reconnecting
    l.server.abort();
    l.until(|a| matches!(a.conn, Conn::Down(_))).await;

    // work continues while clients are cut off
    l.ctx
        .req(
            "POST",
            "/v1/tasks",
            Some(serde_json::json!({"title": "during"})),
        )
        .await;
    let (_, v) = l.ctx.req("GET", "/v1/tasks", None).await;
    let during = v["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["title"] == "during")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // same port back up: the feed resumes with ?cursor= and the log replays
    // exactly the missed event (the snapshot resync alone would not put it
    // in the events ring)
    let listener = tokio::net::TcpListener::bind(l.addr).await.unwrap();
    l.server = serve(listener, l.ctx.router.clone());
    l.until(|a| {
        a.conn == Conn::Live
            && a.store
                .events
                .iter()
                .any(|e| e.subject_id.as_deref() == Some(during.as_str()))
    })
    .await;
    l.until(|a| a.store.tasks.len() == 2).await;
    let seqs: Vec<i64> = l.app.store.events.iter().map(|e| e.seq).collect();
    let mut dedup = seqs.clone();
    dedup.dedup();
    assert_eq!(seqs, dedup, "no event applied twice");
}
