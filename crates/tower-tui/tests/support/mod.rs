//! Fixture builders + a headless renderer for view tests.
#![allow(dead_code)] // each test binary uses a subset

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tower_core::*;
use tower_tui::app::App;
use tower_tui::fmt::Tz;

/// 2026-09-27 10:00:00 UTC — every fixture clock is relative to this.
pub const NOW: i64 = 1_790_503_200_000;

pub fn app() -> App {
    App::new(NOW, Tz::utc())
}

pub fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

pub fn code(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

pub fn render(app: &mut App, w: u16, h: u16) -> Terminal<TestBackend> {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| app.render(f)).unwrap();
    t
}

/// The frame as plain text rows (trailing spaces trimmed).
pub fn text(app: &mut App, w: u16, h: u16) -> String {
    let t = render(app, w, h);
    let buf = t.backend().buffer();
    let mut out = String::new();
    for y in 0..buf.area.height {
        let mut row = String::new();
        for x in 0..buf.area.width {
            row.push_str(buf[(x, y)].symbol());
        }
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}

pub fn agent(name: &str, state: AgentState, machine: &str) -> Agent {
    Agent {
        id: AgentId(format!("A_{name}")),
        name: name.into(),
        kind: "claude".into(),
        machine_id: MachineId(machine.into()),
        pane_id: Some(format!("w1:{name}")),
        workdir: Some("/src/example".into()),
        worktree: None,
        state,
        desired_state: DesiredState::Running,
        permissions: Permissions::Default,
        adopted: false,
        config: serde_json::json!({}),
        created_at: NOW - 3_600_000,
        updated_at: NOW - 60_000,
    }
}

pub fn task(id: &str, title: &str, state: TaskState) -> Task {
    Task {
        id: TaskId(id.into()),
        agent_id: None,
        owner_id: None,
        origin: "cli".into(),
        external_ref: None,
        context_id: None,
        title: title.into(),
        description: None,
        state,
        priority: 0,
        tags: vec![],
        attempt_count: 0,
        max_attempts: 3,
        lease_expires_at: None,
        lease_s: 60,
        target_agent_id: None,
        not_before: None,
        schedule_id: None,
        occurrence_at: None,
        result: None,
        created_at: NOW - 300_000,
        updated_at: NOW - 60_000,
    }
}

pub fn owned(id: &str, title: &str, state: TaskState, owner: &str, lease_in_ms: i64) -> Task {
    Task {
        owner_id: Some(AgentId(format!("A_{owner}"))),
        lease_expires_at: Some(NOW + lease_in_ms),
        ..task(id, title, state)
    }
}

pub fn message(
    id: &str,
    from: &str,
    kind: MessageKind,
    text: &str,
    deadline_in_ms: i64,
) -> Message {
    Message {
        id: MessageId(id.into()),
        task_id: None,
        from_kind: PartyKind::Agent,
        from_id: from.into(),
        to_kind: PartyKind::Human,
        to_id: "me".into(),
        kind,
        parts: vec![Part::text(text)],
        status: MessageStatus::Pending,
        deadline_at: Some(NOW + deadline_in_ms),
        responded_at: None,
        created_at: NOW - 40_000,
    }
}

pub fn machine(name: &str, status: &str, last_seen_ago_ms: Option<i64>) -> Machine {
    Machine {
        id: MachineId(name.into()),
        name: name.into(),
        role: if name == "local" {
            "coordinator".into()
        } else {
            "node".into()
        },
        address: (name != "local").then(|| format!("{name}.lan:8266")),
        status: status.into(),
        last_seen_at: last_seen_ago_ms.map(|a| NOW - a),
        created_at: NOW - 86_400_000,
    }
}

pub fn event(
    seq: i64,
    kind: &str,
    subject: Option<(&str, &str)>,
    payload: serde_json::Value,
) -> Event {
    serde_json::from_value(serde_json::json!({
        "seq": seq,
        "ts": NOW - 1000 * (100 - seq),
        "kind": kind,
        "subject_type": subject.map(|s| s.0),
        "subject_id": subject.map(|s| s.1),
        "payload": payload,
    }))
    .unwrap()
}
