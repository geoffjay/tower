//! The embedded work-loop contract (D§10 `tower contract`): the binary
//! ships `docs/agent-loop.md` so agents can read the contract from any
//! workdir, without the repo cloned.

use tower_server::contract::AGENT_LOOP;

#[test]
fn contract_is_the_work_loop_doc() {
    assert!(AGENT_LOOP.starts_with("# The agent work loop"));
    // the rules and the loop table are the load-bearing sections
    for marker in [
        "You never take work",
        "You own an assigned job exclusively",
        "Declare, heartbeat, report",
        "One job at a time",
        "tower_task_start",
        "tower task start",
        "lease_s / 3",
    ] {
        assert!(AGENT_LOOP.contains(marker), "missing: {marker}");
    }
}

#[test]
fn contract_has_no_repo_relative_links_that_break_outside_the_repo() {
    // printed via `tower contract` in arbitrary workdirs; repo-relative
    // links are fine as text, but the doc must not *require* the repo
    assert!(!AGENT_LOOP.contains("docs/agent-loop.md"));
}
