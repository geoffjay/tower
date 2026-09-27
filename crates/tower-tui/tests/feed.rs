//! Live data (plan T1.2, T4.1): a recorded `/v1/events` stream drives the
//! reducers; replays are idempotent; view-scoped fetches only fire for
//! the open view; the events view filters and follows.

mod support;

use std::collections::HashSet;

use crossterm::event::KeyCode;
use support::*;
use tower_core::*;
use tower_tui::app::{App, Effect, ViewId};
use tower_tui::feed::{FeedMsg, Parser};
use tower_tui::store::{Fetch, Loaded};

const RECORDED: &str = include_str!("fixtures/session.sse");
const F1: &str = "01M3J8HDX39B0SJG1XJ2VT3M89";

/// The recording, parsed in awkward chunk sizes like a real socket.
fn recorded_events() -> Vec<Event> {
    let mut p = Parser::default();
    let mut out = Vec::new();
    for chunk in RECORDED.as_bytes().chunks(97) {
        for f in p.push(chunk) {
            out.extend(f.events());
        }
    }
    out
}

fn fetches(fx: &[Effect]) -> HashSet<Fetch> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Fetch(f) => Some(f.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn recorded_session_drives_the_store() {
    let events = recorded_events();
    assert_eq!(events.len(), 12, "10 non-output events + 2 trimmed outputs");
    let mut a = app();
    let fx = a.on_feed(FeedMsg::Connected);
    assert_eq!(fetches(&fx), HashSet::from([Fetch::All]));
    a.on_loaded(Fetch::All, Ok(vec![]));

    let fx = a.on_feed(FeedMsg::Events(events));
    let task = |id: &str| Fetch::Task(TaskId(id.into()));
    assert_eq!(
        fetches(&fx),
        HashSet::from([
            Fetch::All, // server.started
            task("01M3J8G67NKH7FAED994V0G669"),
            task("01M3J8G68J2N9G7QHPW4VN3YBD"),
            Fetch::Inbox,
            Fetch::Schedules,
            Fetch::Agents,
        ]),
        "no Output/History/TaskDetail: no agent or job detail is open"
    );
    assert_eq!(a.store.seq, Some(12));
    assert_eq!(a.store.events.len(), 12);
    // output payloads can be whole screens; the ring keeps only their size
    let out = a
        .store
        .events
        .iter()
        .find(|e| e.kind == EventKind::AgentOutput)
        .unwrap();
    assert!(out.payload.get("text").is_none());
    assert!(out.payload["bytes"].as_i64().unwrap() > 0);
}

#[test]
fn replay_after_reconnect_is_idempotent() {
    let mut a = app();
    a.on_feed(FeedMsg::Events(recorded_events()));
    let before = a.store.events.len();
    // reconnect: the server replays from our cursor, overlapping rows
    let fx = a.on_feed(FeedMsg::Events(recorded_events()));
    assert!(fx.is_empty());
    assert_eq!(a.store.events.len(), before);
}

fn with_f1() -> App {
    let mut a = app();
    let mut f1 = agent("f1", AgentState::Launching, "local");
    f1.id = AgentId(F1.into());
    a.store.load(Loaded::Agents(vec![f1]));
    a
}

#[test]
fn agent_state_is_patched_in_place() {
    let mut a = with_f1();
    let fx = a.on_feed(FeedMsg::Events(vec![event(
        50,
        "agent.state",
        Some(("agent", F1)),
        serde_json::json!({"from": "launching", "to": "blocked"}),
    )]));
    assert!(fx.is_empty(), "the payload carries the new state");
    assert_eq!(a.store.agents[0].state, AgentState::Blocked);

    // an agent we have never seen: refetch the roster
    let fx = a.on_feed(FeedMsg::Events(vec![event(
        51,
        "agent.state",
        Some(("agent", "A_ghost")),
        serde_json::json!({"from": "idle", "to": "working"}),
    )]));
    assert_eq!(fetches(&fx), HashSet::from([Fetch::Agents]));

    a.on_loaded(Fetch::Agents, Ok(vec![]));
    a.on_feed(FeedMsg::Events(vec![event(
        52,
        "agent.removed",
        Some(("agent", F1)),
        serde_json::json!({"name": "f1"}),
    )]));
    assert!(a.store.agents.is_empty());
}

#[test]
fn output_refetches_only_for_the_open_agent_and_coalesce() {
    let mut a = with_f1();
    let output = |seq| {
        event(
            seq,
            "agent.output",
            Some(("agent", F1)),
            serde_json::json!({"text": "x"}),
        )
    };
    assert!(a.on_feed(FeedMsg::Events(vec![output(60)])).is_empty());

    a.on_key(code(KeyCode::Enter)); // open f1
    let id = AgentId(F1.into());
    let fx = a.on_feed(FeedMsg::Events(vec![output(61), output(62)]));
    assert!(fx.is_empty(), "already in flight from opening the view");
    // the read in flight predates 61/62: it is fetched once more when it lands
    let fx = a.on_loaded(
        Fetch::Output(id.clone()),
        Ok(vec![Loaded::Output {
            agent: id.clone(),
            text: "old".into(),
        }]),
    );
    assert_eq!(fx, vec![Effect::Fetch(Fetch::Output(id.clone()))]);
    let fx = a.on_loaded(
        Fetch::Output(id.clone()),
        Ok(vec![Loaded::Output {
            agent: id.clone(),
            text: "new".into(),
        }]),
    );
    assert!(fx.is_empty(), "nothing arrived meanwhile");

    // leaving the detail stops screen reads
    a.on_key(key('1'));
    assert!(a.on_feed(FeedMsg::Events(vec![output(63)])).is_empty());
}

#[test]
fn message_events_refresh_inbox_and_the_parties_history() {
    let mut a = with_f1();
    a.on_key(code(KeyCode::Enter));
    let id = AgentId(F1.into());
    a.on_loaded(Fetch::History(id.clone()), Ok(vec![]));
    let fx = a.on_feed(FeedMsg::Events(vec![event(
        70,
        "message.created",
        Some(("message", "M9")),
        serde_json::json!({"kind": "question", "from": {"kind": "agent", "id": "f1"}, "to": {"kind": "human", "id": "me"}}),
    )]));
    assert_eq!(
        fetches(&fx),
        HashSet::from([Fetch::Inbox, Fetch::History(id)])
    );
}

#[test]
fn events_view_filters() {
    let mut a = with_f1();
    a.on_feed(FeedMsg::Events(recorded_events()));
    a.on_key(key('4'));
    assert_eq!(a.view, ViewId::Events);
    let kinds = |a: &App| -> Vec<&'static str> {
        a.events
            .visible(&a.store)
            .iter()
            .map(|e| e.kind.as_str())
            .collect()
    };
    assert!(
        !kinds(&a).contains(&"agent.output"),
        "output hidden by default"
    );
    assert_eq!(kinds(&a).len(), 10);

    a.on_key(key('f'));
    for c in "type:task".chars() {
        a.on_key(key(c));
    }
    a.on_key(code(KeyCode::Enter));
    assert_eq!(kinds(&a), ["task.created", "task.created", "task.assigned"]);

    // subject by agent name resolves to the agent's id
    a.on_key(key('c'));
    a.on_key(key('s'));
    a.on_key(key('f'));
    a.on_key(key('1'));
    a.on_key(code(KeyCode::Enter));
    assert_eq!(kinds(&a), ["agent.created", "agent.state"]);
    a.on_key(key('O'));
    assert_eq!(kinds(&a).len(), 4, "+2 output events");
    insta::assert_snapshot!(text(&mut a, 110, 10));
}

#[test]
fn events_view_follows_until_you_scroll() {
    let mut a = app();
    a.on_key(key('4'));
    let evs: Vec<Event> = (1..=5)
        .map(|s| {
            event(
                s,
                "task.created",
                Some(("task", "T1")),
                serde_json::json!({}),
            )
        })
        .collect();
    a.on_feed(FeedMsg::Events(evs));
    assert!(a.events.following());
    a.on_key(key('k')); // pause on the row above the newest
    assert_eq!(a.events.sel, Some(4));
    a.on_feed(FeedMsg::Events(vec![event(
        6,
        "task.created",
        Some(("task", "T1")),
        serde_json::json!({}),
    )]));
    assert_eq!(a.events.sel, Some(4), "new rows don't move a paused view");
    a.on_key(key('G'));
    assert!(a.events.following());
}
