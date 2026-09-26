//! Fixture-driven parsing tests for the herdr JSON envelopes (plan T4.2).
//! Golden files recorded live during spike S1.A (herdr 0.8.2, protocol 20).

use tower_driver::{Harness, HarnessState, ReadSource};

fn fixture(name: &str) -> serde_json::Value {
    let path = format!("tests/fixtures/{name}.json");
    let raw = std::fs::read_to_string(&path).unwrap();
    serde_json::from_str(&raw).unwrap()
}

/// Mirror of the driver's private parse path (kept in one place would be
/// better; these tests pin the envelope shape, so they re-implement the
/// three-line extraction).
#[test]
fn snapshot_envelope_parses_agents_and_panes() {
    let v = fixture("snapshot");
    assert!(
        v.get("error").is_none(),
        "fixture must be a success envelope"
    );
    let snap = &v["result"]["snapshot"];
    let agents = snap["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["name"], "spike-pi");
    assert_eq!(agents[0]["agent"], "pi");
    assert_eq!(agents[0]["agent_status"], "idle");
    let panes = snap["panes"].as_array().unwrap();
    assert_eq!(panes.len(), 2);
}

#[test]
fn read_envelope_output_field() {
    let v = fixture("read");
    let output = v["result"]["output"].as_str().unwrap();
    assert!(output.contains("Thought 412ms"));
}

#[test]
fn error_envelope_shape() {
    // recorded live: {"error":{"code":"agent_not_found","message":"agent target nonexistent-agent not found"},"id":"cli:agent:prompt"}
    let v: serde_json::Value = serde_json::from_str(
        r#"{"error":{"code":"agent_not_found","message":"agent target nonexistent-agent not found"},"id":"cli:agent:prompt"}"#,
    )
    .unwrap();
    assert_eq!(v["error"]["code"], "agent_not_found");
    assert!(v.get("result").is_none());
}

#[test]
fn state_detection_mapping() {
    for (s, expect) in [
        ("idle", HarnessState::Idle),
        ("working", HarnessState::Working),
        ("blocked", HarnessState::Blocked),
        ("done", HarnessState::Done),
        ("unknown", HarnessState::Unknown),
    ] {
        assert_eq!(HarnessState::from_detection(s), Some(expect));
    }
    assert_eq!(HarnessState::from_detection("bogus"), None);
}

#[test]
fn read_source_grammar() {
    assert_eq!(ReadSource::Recent.as_str(), "recent");
    assert_eq!(ReadSource::RecentUnwrapped.as_str(), "recent-unwrapped");
}

/// The trait must be object-safe (driver stored as Arc<dyn Harness>).
#[test]
fn harness_object_safe() {
    let _f: Box<dyn Harness> = Box::new(tower_driver::herdr::HerdrDriver::new());
}
