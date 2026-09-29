//! View tests (plan T1.1, T2.1, T2.2, T3.1, T3.2, T4.2): frames rendered
//! headless with a pinned clock and UTC.

mod support;

use crossterm::event::KeyCode;
use ratatui::style::Color;
use support::*;
use tower_core::*;
use tower_tui::app::{Api, Effect, InputPurpose, ViewId};
use tower_tui::store::{Fetch, Loaded, TaskDetail};

#[test]
fn empty_state_frames() {
    for (name, view) in [
        ("fleet", ViewId::Fleet),
        ("inbox", ViewId::Inbox),
        ("tasks", ViewId::Tasks),
        ("events", ViewId::Events),
        ("machines", ViewId::Machines),
    ] {
        let mut a = app();
        a.view = view;
        insta::assert_snapshot!(format!("empty_{name}"), text(&mut a, 80, 14));
    }
    // agent detail before its first read/history lands
    let mut a = app();
    a.store.load(Loaded::Agents(vec![agent(
        "api",
        AgentState::Idle,
        "local",
    )]));
    a.on_key(code(KeyCode::Enter));
    assert_eq!(a.view, ViewId::Agent);
    insta::assert_snapshot!("empty_agent", text(&mut a, 80, 14));
}

/// A mixed fleet: every state, two machines, jobs in every open state.
fn mixed() -> tower_tui::app::App {
    let mut a = app();
    let mut docs = agent("docs", AgentState::Idle, "local");
    docs.adopted = true;
    docs.worktree = Some("/src/example/.worktrees/docs".into());
    a.store.load(Loaded::Machines(vec![
        machine("local", "online", Some(1000)),
        machine("mini", "online", Some(5000)),
    ]));
    a.store.load(Loaded::Agents(vec![
        agent("api", AgentState::Working, "local"),
        agent("backend", AgentState::Blocked, "mini"),
        docs,
        agent("old", AgentState::Dead, "mini"),
        agent("writer", AgentState::Done, "local"),
    ]));
    a.store.load(Loaded::Tasks(vec![
        owned(
            "T1",
            "implement CSV error column names",
            TaskState::Working,
            "api",
            42_000,
        ),
        owned(
            "T2",
            "migrate schema",
            TaskState::Working,
            "backend",
            30_000,
        ),
        owned(
            "T3",
            "pick a license",
            TaskState::InputRequired,
            "writer",
            50_000,
        ),
        task("T4", "write launch blog post", TaskState::Queued),
        task("T5", "old job", TaskState::Completed),
    ]));
    a.store.load(Loaded::Inbox(vec![message(
        "M1",
        "backend",
        MessageKind::Approval,
        "Allow `cargo publish` for this crate?",
        240_000,
    )]));
    a
}

#[test]
fn fleet_mixed_states_and_queue_banner() {
    let mut a = mixed();
    // blocked = input-required (T3) + working under a blocked owner (T2)
    let q = a.store.queue_counts();
    assert_eq!((q.queued, q.working, q.blocked), (1, 1, 2));
    insta::assert_snapshot!(text(&mut a, 110, 12));
}

fn names(a: &tower_tui::app::App) -> Vec<String> {
    a.fleet
        .rows(&a.store)
        .iter()
        .map(|x| x.name.clone())
        .collect()
}

#[test]
fn fleet_detail_strip_shows_the_full_selected_name() {
    let mut a = mixed();
    a.store.load(Loaded::Agents(vec![agent(
        "a-very-long-agent-name-that-truncates",
        AgentState::Idle,
        "local",
    )]));
    let frame = text(&mut a, 110, 12);
    // the table truncates the name; the detail strip under it does not
    let rows: Vec<&str> = frame.lines().collect();
    assert!(
        rows.iter().any(|r| r.contains("a-very-long-ag ")),
        "{}",
        frame
    );
    let strip = rows
        .iter()
        .find(|r| r.contains("a-very-long-agent-name-that-truncates"))
        .expect("detail strip with the full name");
    assert!(
        strip.contains("local · claude · idle"),
        "strip shows machine/kind/state: {strip}"
    );
}

#[test]
fn fleet_filter_and_sort_keys() {
    let mut a = mixed();
    let cycle = [
        vec!["api"],
        vec!["docs"],
        vec!["backend"],
        vec!["writer"],
        vec!["old"],
        vec!["api", "backend", "docs", "old", "writer"],
    ];
    for want in cycle {
        a.on_key(key('f'));
        assert_eq!(names(&a), want);
    }
    a.on_key(key('m'));
    assert_eq!(names(&a), ["api", "docs", "writer"]);
    a.on_key(key('m'));
    assert_eq!(names(&a), ["backend", "old"]);
    a.on_key(key('m'));
    assert_eq!(names(&a).len(), 5, "third press clears the machine filter");

    a.on_key(key('s'));
    assert_eq!(
        names(&a),
        ["backend", "api", "docs", "writer", "old"],
        "attention order"
    );

    a.on_key(key('f'));
    a.on_key(key('m'));
    a.on_key(key('c'));
    assert_eq!(names(&a).len(), 5);

    // the selection follows the agent, not the row index, across re-sorts
    a.on_key(key('s')); // machine sort
    a.on_key(key('s')); // name sort
    a.on_key(key('j'));
    assert_eq!(a.fleet.sel, Some(AgentId("A_backend".into())));
    a.on_key(key('s'));
    assert_eq!(a.fleet.sel, Some(AgentId("A_backend".into())));
}

#[test]
fn palette_filter_rejects_unknown_values() {
    let mut a = mixed();
    a.on_key(key(':'));
    for c in "filter state=blocked".chars() {
        a.on_key(key(c));
    }
    a.on_key(code(KeyCode::Enter));
    assert_eq!(names(&a), ["backend"]);
    a.on_key(key(':'));
    for c in "filter state=sleepy".chars() {
        a.on_key(key(c));
    }
    a.on_key(code(KeyCode::Enter));
    assert!(a.toast.as_ref().unwrap().error);
    assert_eq!(names(&a), ["backend"], "a bad filter leaves the old one");
}

fn open_api(a: &mut tower_tui::app::App) -> Vec<Effect> {
    a.on_key(code(KeyCode::Enter)) // first row: api
}

#[test]
fn agent_detail_passes_ansi_through_and_tails() {
    let mut a = mixed();
    let fx = open_api(&mut a);
    let id = AgentId("A_api".into());
    assert_eq!(
        fx,
        vec![
            Effect::Fetch(Fetch::Output(id.clone())),
            Effect::Fetch(Fetch::History(id.clone()))
        ]
    );
    let mut screen: String = (1..=40).map(|i| format!("line {i}\n")).collect();
    screen.push_str("\x1b[31mred alert\x1b[0m plain\n\n\n\n");
    a.on_loaded(
        Fetch::Output(id.clone()),
        Ok(vec![Loaded::Output {
            agent: id.clone(),
            text: screen,
        }]),
    );
    let t = render(&mut a, 80, 20);
    let buf = t.backend().buffer();
    let rows: Vec<String> = (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
        .collect();
    let y = rows
        .iter()
        .position(|r| r.contains("red alert plain"))
        .expect("tail shows the last line") as u16;
    assert!(rows
        .iter()
        .all(|r| !r.contains('\x1b') && !r.contains("[31m")));
    // trailing blank rows dropped: the last real line sits right above
    // the output box's bottom border
    assert!(rows[y as usize + 1].contains('└'));
    let x = rows[y as usize].find("red").unwrap();
    let x = rows[y as usize][..x].chars().count() as u16;
    assert_eq!(buf[(x, y)].fg, Color::Red);
    assert_eq!(buf[(x + 10, y)].fg, Color::Reset, "`plain` is unstyled");

    // scrolling up leaves the tail; G follows again
    a.on_key(key('k'));
    assert_eq!(a.agent.as_ref().unwrap().scroll, 1);
    a.on_key(key('G'));
    assert_eq!(a.agent.as_ref().unwrap().scroll, 0);
}

#[test]
fn agent_detail_prompt_input_and_keys() {
    let mut a = mixed();
    open_api(&mut a);
    a.on_key(key('i'));
    assert_eq!(
        a.input.as_ref().map(|(p, _)| p.clone()),
        Some(InputPurpose::Prompt {
            agent: "api".into()
        })
    );
    // keys go to the input while it is focused (`q` doesn't quit)
    for c in "quick check".chars() {
        a.on_key(key(c));
    }
    assert!(!a.quit);
    let fx = a.on_key(code(KeyCode::Enter));
    assert_eq!(
        fx,
        vec![Effect::Api(Api::Prompt {
            agent: "api".into(),
            text: "quick check".into()
        })]
    );
    assert_eq!(
        a.on_key(key('x')),
        vec![Effect::Api(Api::Interrupt {
            agent: "api".into()
        })]
    );
    assert_eq!(
        a.on_key(key('o')),
        vec![Effect::Attach {
            target: "w1:api".into()
        }]
    );
    a.on_key(code(KeyCode::Esc));
    assert_eq!(a.view, ViewId::Fleet);
}

#[test]
fn inbox_replies_by_kind() {
    let mut a = mixed();
    a.store.load(Loaded::Inbox(vec![
        message(
            "M1",
            "backend",
            MessageKind::Approval,
            "Allow `cargo publish`?",
            240_000,
        ),
        message(
            "M2",
            "api",
            MessageKind::Question,
            "Which approach?",
            30_000,
        ),
    ]));
    a.on_key(key('2'));
    assert_eq!(a.view, ViewId::Inbox);
    insta::assert_snapshot!(text(&mut a, 90, 16));

    let respond = |id: &str, approve, text: Option<&str>| {
        vec![Effect::Api(Api::Respond {
            msg: MessageId(id.into()),
            approve,
            text: text.map(str::to_string),
        })]
    };
    assert_eq!(a.on_key(key('y')), respond("M1", Some(true), None));
    assert_eq!(a.on_key(key('n')), respond("M1", Some(false), None));
    assert!(a.on_key(key('r')).is_empty());
    assert!(a.input.is_none(), "approvals take y/n, not text");
    a.on_key(key('j'));
    assert_eq!(a.on_key(key('y')), respond("M2", None, Some("yes")));
    a.on_key(key('r'));
    for c in "the second one".chars() {
        a.on_key(key(c));
    }
    assert_eq!(
        a.on_key(code(KeyCode::Enter)),
        respond("M2", None, Some("the second one"))
    );
}

#[test]
fn lease_countdowns_follow_the_clock() {
    let mut a = mixed();
    a.on_key(key('3'));
    let lease_of = |a: &mut tower_tui::app::App, title: &str| -> String {
        let frame = text(a, 120, 30);
        let row = frame
            .lines()
            .find(|l| l.contains(title))
            .unwrap()
            .to_string();
        row.split_whitespace().nth(3).unwrap().to_string()
    };
    assert_eq!(lease_of(&mut a, "implement CSV"), "42s");
    assert_eq!(lease_of(&mut a, "pick a license"), "held");
    a.tick(NOW + 30_000);
    assert_eq!(lease_of(&mut a, "implement CSV"), "12s");
    a.tick(NOW + 42_000);
    assert_eq!(lease_of(&mut a, "implement CSV"), "expired");
}

#[test]
fn queue_shows_reservations() {
    let mut a = mixed();
    let mut reserved = task("T6", "nightly cleanup", TaskState::Queued);
    reserved.target_agent_id = Some(AgentId("A_backend".into()));
    let mut later = task("T7", "dependency audit", TaskState::Queued);
    later.target_agent_id = Some(AgentId("A_api".into()));
    later.not_before = Some(NOW + 300_000);
    a.store.load(Loaded::TasksDelta(vec![reserved, later]));
    a.on_key(key('3'));
    let frame = text(&mut a, 120, 30);
    assert!(frame.contains("→backend when available"), "{frame}");
    assert!(frame.contains("→api in 5m00s"), "{frame}");
}

#[test]
fn task_detail_renders_the_trail() {
    let mut a = mixed();
    a.on_key(key('3'));
    let fx = a.on_key(code(KeyCode::Enter)); // owned pane, soonest lease: T2
    let id = TaskId("T2".into());
    assert_eq!(fx, vec![Effect::Fetch(Fetch::TaskDetail(id.clone()))]);
    let trail = vec![
        event(
            10,
            "task.created",
            Some(("task", "T2")),
            serde_json::json!({"task_id": "T2"}),
        ),
        event(
            11,
            "task.assigned",
            Some(("task", "T2")),
            serde_json::json!({"task_id": "T2", "owner_id": "A_api", "by": "me"}),
        ),
        event(
            12,
            "task.leased_out",
            Some(("task", "T2")),
            serde_json::json!({"prior_owner": "A_api", "state": "queued", "attempt_count": 1, "max_attempts": 3}),
        ),
        event(
            13,
            "task.assigned",
            Some(("task", "T2")),
            serde_json::json!({"task_id": "T2", "owner_id": "A_backend", "by": "dispatch"}),
        ),
        event(
            14,
            "task.status",
            Some(("task", "T2")),
            serde_json::json!({"state": "working", "owner_id": "A_backend"}),
        ),
    ];
    let task = a.store.tasks[&id].clone();
    a.on_loaded(
        Fetch::TaskDetail(id),
        Ok(vec![Loaded::TaskDetail(Box::new(TaskDetail {
            task,
            trail,
            messages: vec![],
        }))]),
    );
    insta::assert_snapshot!(text(&mut a, 120, 30));
    a.on_key(code(KeyCode::Esc));
    assert!(a.tasks.detail.is_none());
}

#[test]
fn cancel_needs_a_second_press() {
    let mut a = mixed();
    a.on_key(key('3'));
    a.on_key(key('l')); // queue pane
    assert!(a.on_key(key('x')).is_empty());
    assert_eq!(
        a.on_key(key('x')),
        vec![Effect::Api(Api::TaskCancel {
            task: TaskId("T4".into())
        })]
    );
    // any other key in between disarms it
    a.on_key(key('x'));
    a.on_key(key('j'));
    assert!(a.on_key(key('x')).is_empty());
}

#[test]
fn machines_render_local_and_an_offline_node() {
    let mut a = mixed();
    a.store.load(Loaded::Machines(vec![
        machine("local", "online", Some(2_000)),
        machine("mini", "offline", Some(2 * 3_600_000)),
    ]));
    a.on_key(key('5'));
    insta::assert_snapshot!(text(&mut a, 90, 6));
}

#[test]
fn palette_commands() {
    let mut a = mixed();
    let run = |a: &mut tower_tui::app::App, cmd: &str| {
        a.on_key(key(':'));
        for c in cmd.chars() {
            a.on_key(key(c));
        }
        a.on_key(code(KeyCode::Enter))
    };
    assert_eq!(
        run(&mut a, "spawn scout pi"),
        vec![Effect::Api(Api::Spawn {
            name: "scout".into(),
            kind: Some("pi".into()),
            workdir: None
        })]
    );
    assert_eq!(
        run(&mut a, "reserve T4 backend"),
        vec![Effect::Api(Api::TaskAssign {
            task: TaskId("T4".into()),
            agent: "backend".into(),
            when_available: true
        })]
    );
    // `T` prefixes several jobs: refuse rather than guess
    assert!(run(&mut a, "cancel T").is_empty());
    assert!(a.toast.as_ref().unwrap().text.contains("matches 5 jobs"));
    assert!(run(&mut a, "frobnicate").is_empty());
    assert!(a.toast.as_ref().unwrap().error);
    run(&mut a, "agent backend");
    assert_eq!(a.view, ViewId::Agent);
    run(&mut a, "q");
    assert!(a.quit);
}

#[test]
fn long_input_scrolls_to_keep_the_cursor_visible() {
    let mut a = mixed();
    a.on_key(key('i'));
    let long: String = (0..90).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    for c in long.chars() {
        a.on_key(key(c));
    }
    let frame = text(&mut a, 60, 8);
    let bottom = frame.lines().last().unwrap();
    assert!(bottom.starts_with("prompt api › "), "{bottom}");
    assert!(bottom.ends_with(&long[long.len() - 20..]), "{bottom}");
}

#[test]
fn approval_context_shows_the_screen_tail_not_json() {
    let mut a = mixed();
    let mut m = message(
        "M1",
        "c1",
        MessageKind::Approval,
        "c1 is blocked and waiting for input.",
        240_000,
    );
    m.parts.push(Part::data(serde_json::json!({
        "context": "(eval): shell noise\n\n\n Accessing workspace:\n\n /private/tmp/x\n\n ❯ 1. Yes, I trust this folder\n   2. No, exit\n\n Enter to confirm · Esc to cancel\n"
    })));
    a.store.load(Loaded::Inbox(vec![m]));
    a.on_key(key('2'));
    let frame = text(&mut a, 80, 12);
    assert!(
        !frame.contains("\\n") && !frame.contains("{\"context\""),
        "{frame}"
    );
    // the dialog (bottom of the capture) is what fits, not the shell noise
    assert!(frame.contains("❯ 1. Yes, I trust this folder"), "{frame}");
    assert!(frame.contains("Enter to confirm"), "{frame}");
    assert!(!frame.contains("shell noise"), "{frame}");
}
