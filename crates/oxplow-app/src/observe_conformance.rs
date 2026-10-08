//! The observe conformance suite (`.context/agent-model.md` "Observe
//! conformance"): what core must record of any harness's session, whatever
//! its hooks look like on the wire. Run after a harness's scripted session
//! — live, over the fake harness (`oxplow-harness-fake`, the control
//! plane's `tests/observe_conformance.rs`), or replayed from a real
//! harness's recorded hooks — it reads the thread's events and checks the
//! canonical stream: the session started under the harness's key, a prompt
//! opened a turn, the edit was requested and finished with its path, the
//! turn ended, and the tokens the harness reported were read.
//!
//! Each check is a [`Finding`] when it fails; an empty list passes.

use oxplow_domain::{StoredEvent, ThreadId};
use serde_json::Value;

pub use crate::work_items_conformance::Finding;

/// What the session did, which the events must show.
#[derive(Debug, Clone)]
pub struct Expect<'a> {
    /// The harness's registry key.
    pub harness: &'a str,
    pub thread: ThreadId,
    /// The path it edited, relative to the worktree.
    pub edited: &'a str,
    /// The input and output tokens it reported.
    pub tokens: (u64, u64),
}

/// The thread's events, oldest first.
async fn events_of(svc: &crate::Services, thread: ThreadId) -> Vec<StoredEvent> {
    svc.event_log_store
        .read_after(0, 100_000)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.envelope.anchors.thread_id == Some(thread))
        .collect()
}

/// Run the checks over what `svc` recorded.
pub async fn suite(svc: &crate::Services, expect: &Expect<'_>) -> Vec<Finding> {
    let events = events_of(svc, expect.thread).await;
    let payloads = |event_type: &str| -> Vec<&Value> {
        events
            .iter()
            .filter(|e| e.envelope.event_type == event_type)
            .map(|e| &e.envelope.payload)
            .collect()
    };
    let mut findings = Vec::new();
    let mut check = |check: &'static str, ok: bool, message: String| {
        if !ok {
            findings.push(Finding { check, message });
        }
    };
    let started = payloads("agent.session.started");
    check(
        "session_started",
        started.iter().any(|p| p["harness"] == expect.harness),
        format!(
            "no agent.session.started naming `{}`: {started:?}",
            expect.harness
        ),
    );
    check(
        "prompt_submitted",
        !payloads("agent.prompt.submitted").is_empty(),
        "no agent.prompt.submitted".into(),
    );
    check(
        "turn_started",
        !payloads("agent.turn.started").is_empty(),
        "no agent.turn.started".into(),
    );
    let requested = payloads("agent.tool.requested");
    check(
        "tool_requested",
        requested
            .iter()
            .any(|p| p["tool"] == "Edit" && p["path"] == expect.edited),
        format!(
            "no agent.tool.requested for an Edit of {}: {requested:?}",
            expect.edited
        ),
    );
    let finished = payloads("agent.tool.finished");
    check(
        "tool_finished",
        finished
            .iter()
            .any(|p| p["tool"] == "Edit" && p["path"] == expect.edited && p["ok"] == true),
        format!(
            "no successful agent.tool.finished for the Edit of {}: {finished:?}",
            expect.edited
        ),
    );
    let ended = payloads("agent.turn.ended");
    check(
        "turn_ended",
        ended.iter().any(|p| p["outcome"] == "completed"),
        format!("no completed agent.turn.ended: {ended:?}"),
    );
    let reported = payloads("agent.tokens.reported");
    let total = |kind: &str| -> u64 {
        reported
            .iter()
            .flat_map(|p| p["counts"].as_array().cloned().unwrap_or_default())
            .filter(|c| c["kind"] == kind)
            .filter_map(|c| c["value"].as_u64())
            .sum()
    };
    check(
        "tokens_reported",
        (total("input"), total("output")) == expect.tokens,
        format!(
            "agent.tokens.reported counted {:?}, not {:?}",
            (total("input"), total("output")),
            expect.tokens
        ),
    );
    findings
}
