//! Round-trip serde tests for core wire shapes (plan T2.1).

use tower_core::*;

#[test]
fn agent_roundtrip() {
    let a = Agent {
        id: AgentId::from("01J9X0"),
        name: "research".into(),
        kind: "pi".into(),
        machine_id: MachineId::from("local"),
        pane_id: Some("w3:p3".into()),
        workdir: Some("~/Projects/example".into()),
        worktree: None,
        state: AgentState::Working,
        desired_state: DesiredState::Running,
        permissions: Permissions::AcceptEdits,
        adopted: true,
        config: serde_json::json!({}),
        created_at: 1,
        updated_at: 2,
    };
    let v = serde_json::to_value(&a).unwrap();
    assert_eq!(v["state"], "working");
    assert_eq!(v["permissions"], "accept-edits");
    let back: Agent = serde_json::from_value(v).unwrap();
    assert_eq!(back.name, a.name);
    assert_eq!(back.state, a.state);
}

#[test]
fn task_pool_fields_roundtrip() {
    let t = Task {
        id: TaskId::from("t_1"),
        agent_id: None,
        owner_id: Some(AgentId::from("a_1")),
        origin: "local".into(),
        external_ref: None,
        context_id: None,
        title: "implement thing".into(),
        description: None,
        state: TaskState::InputRequired,
        priority: 2,
        tags: vec!["rust".into()],
        attempt_count: 0,
        max_attempts: 3,
        lease_expires_at: Some(123),
        result: None,
        created_at: 1,
        updated_at: 2,
    };
    let v = serde_json::to_value(&t).unwrap();
    assert_eq!(v["state"], "input-required");
    let back: Task = serde_json::from_value(v).unwrap();
    assert_eq!(back.owner_id, t.owner_id);
}

#[test]
fn message_parts_roundtrip() {
    let m = Message {
        id: MessageId::from("m_1"),
        task_id: None,
        from_kind: PartyKind::Agent,
        from_id: "backend".into(),
        to_kind: PartyKind::Human,
        to_id: "me".into(),
        kind: MessageKind::Approval,
        parts: vec![Part::text("Allow cargo publish?")],
        status: MessageStatus::Pending,
        deadline_at: Some(60_000),
        responded_at: None,
        created_at: 1,
    };
    let v = serde_json::to_value(&m).unwrap();
    assert_eq!(v["kind"], "approval");
    assert_eq!(v["parts"][0]["text"], "Allow cargo publish?");
    let back: Message = serde_json::from_value(v).unwrap();
    assert_eq!(back.parts.len(), 1);
}

#[test]
fn event_envelope_shape() {
    let e = Event {
        seq: 42,
        ts: 1000,
        kind: EventKind::AgentStateChange,
        subject_type: Some("agent".into()),
        subject_id: Some("a_1".into()),
        payload: serde_json::json!({"from": "idle", "to": "working"}),
    };
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["kind"], "agent.state");
    assert_eq!(v["seq"], 42);
    let back: Event = serde_json::from_value(v).unwrap();
    assert_eq!(back.kind, EventKind::AgentStateChange);
}

#[test]
fn error_envelope_codes() {
    assert_eq!(ErrorCode::Conflict.as_str(), "conflict");
    assert_eq!(ErrorCode::Conflict.http_status(), 409);
    let e = TowerError::not_found("agent x");
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["code"], "not_found");
    assert_eq!(v["message"], "agent x");
}
