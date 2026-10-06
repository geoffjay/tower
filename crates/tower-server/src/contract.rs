//! The agent work-loop contract, embedded at build time.
//!
//! `docs/agent-loop.md` is the single source of truth; `include_str!` bakes
//! it into the binary so `tower contract` prints it anywhere — agents run
//! in arbitrary workdirs without the repo cloned. Docs edits ship with the
//! next build.

/// The full work-loop contract (D§10: `tower contract`).
pub const AGENT_LOOP: &str = include_str!("../../../docs/agent-loop.md");
