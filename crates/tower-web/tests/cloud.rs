//! Agent cloud end to end over a fixture source (phase 4 T1.1, T2.1,
//! T2.2, T3.1): the HTTP render, and the live WebSocket protocol the
//! browser runtime speaks (`topcoat-runtime` subprotocol, JSON frames).

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use parking_lot::Mutex;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower_core::{
    Agent, AgentId, AgentState, DesiredState, Event, EventKind, Machine, MachineId, Message,
    MessageId, MessageKind, MessageStatus, PartyKind, Permissions, Task, TaskId, TaskState,
};
use tower_web::{Snapshot, UiSource};

const NOW: i64 = 1_790_000_000_000;

struct Fixture {
    snap: Mutex<Snapshot>,
    events: Mutex<Vec<Event>>,
    head: tokio::sync::watch::Sender<i64>,
    now: AtomicI64,
}

impl Fixture {
    fn new(snap: Snapshot) -> Arc<Self> {
        Arc::new(Self {
            snap: Mutex::new(snap),
            events: Mutex::new(vec![]),
            head: tokio::sync::watch::channel(0).0,
            now: AtomicI64::new(NOW),
        })
    }

    fn push(&self, kind: EventKind, agent: Option<&str>, payload: serde_json::Value) {
        let mut events = self.events.lock();
        let seq = events.len() as i64 + 1;
        events.push(Event {
            seq,
            ts: self.now.load(Ordering::SeqCst),
            kind,
            subject_type: agent.map(|_| "agent".into()),
            subject_id: agent.map(|a| format!("A_{a}")),
            payload,
        });
        drop(events);
        self.head.send_replace(seq);
    }

    /// What the server does on a state change: row first, then the event.
    fn set_state(&self, name: &str, state: AgentState) {
        let mut snap = self.snap.lock();
        let a = snap.agents.iter_mut().find(|a| a.name == name).unwrap();
        let from = a.state;
        a.state = state;
        drop(snap);
        self.push(
            EventKind::AgentStateChange,
            Some(name),
            serde_json::json!({"from": from.as_str(), "to": state.as_str()}),
        );
    }
}

#[async_trait::async_trait]
impl UiSource for Fixture {
    async fn snapshot(&self) -> anyhow::Result<Snapshot> {
        Ok(self.snap.lock().clone())
    }

    async fn events_since(&self, cursor: i64, limit: i64) -> anyhow::Result<Vec<Event>> {
        Ok(self
            .events
            .lock()
            .iter()
            .filter(|e| e.seq > cursor)
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn cursor_at(&self, ts: i64) -> anyhow::Result<i64> {
        let events = self.events.lock();
        Ok(events
            .iter()
            .find(|e| e.ts >= ts)
            .map_or(events.len() as i64, |e| e.seq - 1))
    }

    fn head(&self) -> tokio::sync::watch::Receiver<i64> {
        self.head.subscribe()
    }

    fn now(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

fn machine(id: &str, role: &str, status: &str) -> Machine {
    Machine {
        id: MachineId(id.into()),
        name: id.into(),
        role: role.into(),
        address: None,
        status: status.into(),
        last_seen_at: Some(NOW),
        created_at: NOW - 86_400_000,
    }
}

fn agent(name: &str, state: AgentState) -> Agent {
    Agent {
        id: AgentId(format!("A_{name}")),
        name: name.into(),
        kind: "pi".into(),
        machine_id: MachineId("local".into()),
        pane_id: None,
        workdir: None,
        worktree: None,
        state,
        desired_state: DesiredState::Running,
        permissions: Permissions::Default,
        adopted: false,
        config: serde_json::json!({}),
        created_at: NOW - 3_600_000,
        updated_at: NOW,
    }
}

fn job(id: &str, owner: Option<&str>, state: TaskState, lease_in_ms: Option<i64>) -> Task {
    Task {
        id: TaskId(id.into()),
        agent_id: None,
        owner_id: owner.map(|o| AgentId(format!("A_{o}"))),
        origin: "cli".into(),
        external_ref: None,
        context_id: None,
        title: format!("job {id}"),
        description: None,
        state,
        priority: 0,
        tags: vec![],
        attempt_count: 0,
        max_attempts: 3,
        lease_expires_at: lease_in_ms.map(|l| NOW + l),
        lease_s: 120,
        target_agent_id: None,
        not_before: None,
        schedule_id: None,
        occurrence_at: None,
        result: None,
        created_at: NOW - 60_000,
        updated_at: NOW,
    }
}

fn question_from(name: &str) -> Message {
    Message {
        id: MessageId(format!("m_{name}")),
        task_id: None,
        from_kind: PartyKind::Agent,
        from_id: name.into(),
        to_kind: PartyKind::Human,
        to_id: "operator".into(),
        kind: MessageKind::Question,
        parts: vec![],
        status: MessageStatus::Pending,
        deadline_at: None,
        responded_at: None,
        created_at: NOW,
    }
}

fn roster(n: usize) -> Snapshot {
    Snapshot {
        machines: vec![machine("local", "coordinator", "online")],
        agents: (0..n)
            .map(|i| agent(&format!("ag{i:02}"), AgentState::Idle))
            .collect(),
        ..Default::default()
    }
}

async fn get(app: &axum::Router, uri: &str) -> (axum::http::StatusCode, String, String) {
    let req = axum::http::Request::get(uri)
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = tower::ServiceExt::oneshot(app.clone(), req).await.unwrap();
    let status = resp.status();
    let ctype = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, ctype, String::from_utf8(body.to_vec()).unwrap())
}

/// The `<g class=…>` of one agent's point.
fn point_class(html: &str, name: &str) -> String {
    let at = html
        .find(&format!("id=\"a-A_{name}\""))
        .unwrap_or_else(|| panic!("no point for {name}"));
    let rest = &html[at..];
    let c = rest.find("class=\"").unwrap() + 7;
    rest[c..c + rest[c..].find('"').unwrap()].to_string()
}

#[tokio::test]
async fn roster_renders_one_point_per_agent_server_side() {
    let fx = Fixture::new(roster(12));
    let app = tower_web::router(fx.clone());
    let (status, _, html) = get(&app, "/ui").await;
    assert_eq!(status, 200);
    assert_eq!(html.matches("<g id=\"a-").count(), 12);
    for i in 0..12 {
        assert!(html.contains(&format!("id=\"a-A_ag{i:02}\"")));
    }

    // the runtime script comes from memory, at the URL the page references
    assert!(html.contains("/ui/assets/topcoat-runtime-0.9.0.js"));
    let (status, ctype, js) = get(&app, "/ui/assets/topcoat-runtime-0.9.0.js").await;
    assert_eq!(status, 200);
    assert!(ctype.starts_with("text/javascript"));
    assert!(js.len() > 10_000);
}

#[tokio::test]
async fn mixed_states_render_color_halo_and_brightness() {
    let states = [
        ("w", AgentState::Working),
        ("b", AgentState::Blocked),
        ("i", AgentState::Idle),
        ("d", AgentState::Done),
        ("x", AgentState::Dead),
        ("l", AgentState::Launching),
        ("u", AgentState::Unknown),
        ("asker", AgentState::Idle),
        ("faulty", AgentState::Idle),
        ("owner", AgentState::Working),
    ];
    let mut snap = Snapshot {
        machines: vec![machine("local", "coordinator", "online")],
        agents: states.iter().map(|(n, s)| agent(n, *s)).collect(),
        ..Default::default()
    };
    snap.pending_from_agents.push(question_from("asker"));
    snap.open_tasks = vec![
        job("t1", None, TaskState::Queued, None),
        job("t2", Some("owner"), TaskState::Working, Some(100_000)),
        job("t3", Some("b"), TaskState::Working, Some(100_000)),
    ];
    let fx = Fixture::new(snap);
    fx.push(
        EventKind::TaskFailed,
        None,
        serde_json::json!({"task_id": "t9", "owner_id": "A_faulty"}),
    );
    fx.push(
        EventKind::AgentOutput,
        Some("owner"),
        serde_json::json!({"text": "compiling"}),
    );
    let app = tower_web::router(fx.clone());
    tokio::time::sleep(Duration::from_millis(100)).await; // follower catches up
    let (_, _, html) = get(&app, "/ui").await;

    assert_eq!(point_class(&html, "w"), "pt working");
    assert_eq!(point_class(&html, "b"), "pt blocked needs");
    assert_eq!(point_class(&html, "i"), "pt idle");
    assert_eq!(point_class(&html, "d"), "pt done");
    assert_eq!(point_class(&html, "x"), "pt dead");
    assert_eq!(point_class(&html, "l"), "pt launching");
    assert_eq!(point_class(&html, "u"), "pt unknown");
    assert_eq!(
        point_class(&html, "asker"),
        "pt idle needs",
        "pending question"
    );
    assert_eq!(point_class(&html, "faulty"), "pt idle fault");
    assert!(html.contains("2 need you: asker, b · 1 in inbox"));
    // blocked owner's job counts as blocked, like the TUI banner
    assert!(html.contains("<b>1</b> queued · <b>1</b> working · <b class=\"warn\">1</b> blocked"));
    // brightness: dead is dimmest; a working agent with fresh output is full
    assert!(html.contains("fill-opacity: 0.35"));
    // bigger point for the active agent
    let owner = &html[html.find("id=\"a-A_owner\"").unwrap()..];
    assert!(
        owner.contains("r: 8.6px; fill-opacity: 1.00"),
        "{}",
        &owner[..600]
    );
}

// ---- live protocol -------------------------------------------------------

async fn serve(app: axum::Router, addr: SocketAddr) -> tokio::task::JoinHandle<()> {
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    })
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(addr: SocketAddr, signals: serde_json::Value) -> Ws {
    let mut req = format!("ws://{addr}/ui").into_client_request().unwrap();
    req.headers_mut()
        .insert("sec-websocket-protocol", "topcoat-runtime".parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    ws.send(WsMessage::text(
        serde_json::json!({"run": 1, "signals": signals}).to_string(),
    ))
    .await
    .unwrap();
    ws
}

/// Next frame whose html satisfies `pred`, within `within`.
async fn frame_where(ws: &mut Ws, within: Duration, pred: impl Fn(&str) -> bool) -> String {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let msg = tokio::time::timeout_at(deadline, ws.next())
            .await
            .expect("no matching frame in time")
            .unwrap()
            .unwrap();
        let WsMessage::Text(t) = msg else { continue };
        let v: serde_json::Value = serde_json::from_str(t.as_str()).unwrap();
        if let Some(html) = v["html"].as_str()
            && pred(html)
        {
            return html.to_string();
        }
    }
}

fn free_addr() -> SocketAddr {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap()
}

#[tokio::test]
async fn events_drive_every_widget_within_two_seconds() {
    let fx = Fixture::new(roster(10));
    let addr = free_addr();
    let _server = serve(tower_web::router(fx.clone()), addr).await;
    let mut ws = connect(addr, serde_json::json!({})).await;
    let first = frame_where(&mut ws, Duration::from_secs(5), |h| h.contains("a-A_ag03")).await;
    assert_eq!(point_class(&first, "ag03"), "pt idle");

    // working → blocked → idle flips, each within 2 s
    for (state, class) in [
        (AgentState::Working, "pt working"),
        (AgentState::Blocked, "pt blocked needs"),
        (AgentState::Idle, "pt idle"),
    ] {
        fx.set_state("ag03", state);
        let html = frame_where(&mut ws, Duration::from_secs(2), |h| {
            h.contains("a-A_ag03") && point_class(h, "ag03") == class
        })
        .await;
        assert!(html.contains("agent.state"), "ribbon shows the event");
    }

    // a burst: queue bar, machine strip, ribbon all catch up
    {
        let mut snap = fx.snap.lock();
        snap.open_tasks = (0..5)
            .map(|i| job(&format!("q{i}"), None, TaskState::Queued, None))
            .collect();
        snap.machines[0].status = "offline".into();
    }
    for i in 0..40 {
        fx.push(
            EventKind::AgentOutput,
            Some(&format!("ag{:02}", i % 10)),
            serde_json::json!({"text": format!("line {i}")}),
        );
    }
    fx.push(
        EventKind::TaskCreated,
        None,
        serde_json::json!({"task_id": "q4", "title": "last one"}),
    );
    frame_where(&mut ws, Duration::from_secs(2), |h| {
        h.contains("<b>5</b> queued") && h.contains("chip off") && h.contains("last one")
    })
    .await;
}

#[tokio::test]
async fn reconnect_after_server_restart_has_each_point_once() {
    let fx = Fixture::new(roster(8));
    let addr = free_addr();
    let server = serve(tower_web::router(fx.clone()), addr).await;
    let mut ws = connect(addr, serde_json::json!({})).await;
    frame_where(&mut ws, Duration::from_secs(5), |h| h.contains("a-A_ag07")).await;

    server.abort();
    let _ = server.await;
    drop(ws);
    fx.set_state("ag02", AgentState::Working); // changed while down

    let _server = serve(tower_web::router(fx.clone()), addr).await;
    let mut ws = connect(addr, serde_json::json!({})).await;
    let html = frame_where(&mut ws, Duration::from_secs(5), |h| h.contains("a-A_ag07")).await;
    assert_eq!(html.matches("<g id=\"a-").count(), 8);
    assert_eq!(point_class(&html, "ag02"), "pt working");
}

/// The page's `selected` signal id, from the HTTP render.
fn selected_signal_id(html: &str) -> String {
    let marker = "::topcoat::signal({&quot;t&quot;:&quot;signal&quot;,&quot;id&quot;:&quot;";
    let at = html.find(marker).expect("signal marker") + marker.len();
    html[at..at + html[at..].find("&quot;").unwrap()].to_string()
}

#[tokio::test]
async fn panel_follows_the_selection_and_counts_the_lease_down() {
    let mut snap = roster(3);
    snap.open_tasks = vec![job("t1", Some("ag01"), TaskState::Working, Some(90_000))];
    let fx = Fixture::new(snap);
    fx.push(
        EventKind::AgentOutput,
        Some("ag01"),
        serde_json::json!({"text": "step 1\nstep 2\n"}),
    );
    let app = tower_web::router(fx.clone());
    let (_, _, html) = get(&app, "/ui").await;
    assert!(html.contains("panel closed"), "nothing selected");
    let sig = selected_signal_id(&html);

    let addr = free_addr();
    let _server = serve(app, addr).await;
    let mut ws = connect(addr, serde_json::json!({ sig.clone(): "A_ag01" })).await;
    let panel = frame_where(&mut ws, Duration::from_secs(5), |h| {
        h.contains("id=\"panel\"") && h.contains("ag01")
    })
    .await;
    assert!(panel.contains("job t1 · working"));
    assert!(panel.contains("lease 1:30"));
    assert!(panel.contains("step 1\nstep 2"), "output snippet");

    // the countdown ticks from lease_expires_at
    fx.now.fetch_add(5_000, Ordering::SeqCst);
    frame_where(&mut ws, Duration::from_secs(3), |h| {
        h.contains("lease 1:25")
    })
    .await;

    // live state while open
    fx.set_state("ag01", AgentState::Blocked);
    frame_where(&mut ws, Duration::from_secs(2), |h| {
        h.contains("id=\"panel\"") && h.contains("needs you")
    })
    .await;

    // an id that is not an agent renders no panel
    let mut ws = connect(addr, serde_json::json!({ sig: "<script>" })).await;
    frame_where(&mut ws, Duration::from_secs(5), |h| {
        h.contains("panel closed")
    })
    .await;
}
