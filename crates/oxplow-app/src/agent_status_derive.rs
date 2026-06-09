//! Derive a thread's working/waiting status by replaying its hook
//! event log. Mirrors the proven state machine from
//! `src/session/agent-status.ts` on main, ported to operate over
//! `oxplow_domain::HookEvent` rows.
//!
//! The renderer's dot only distinguishes "working" vs "waiting", but
//! we return the richer `AgentStatusState` so the same derivation can
//! drive other UI surfaces. Mapping at the IPC boundary collapses
//! `Running` → working and everything else → waiting.
//!
//! ## Why derive instead of read agent_status
//!
//! The agent_status row is updated by `HookIngestService` on every
//! state-changing hook. Bugs in that pipeline (a missed Stop, a
//! mis-routed SubagentStop, a stale row from a previous boot) make
//! the indicator drift from reality. The hook event log is the
//! authoritative record of what Claude Code emitted; deriving status
//! from it matches the source of truth and self-heals when the
//! sidecar table goes wrong.

use oxplow_domain::{AgentStatusState, HookEvent, HookKind};

/// Replay `events` (which may arrive in any order) and return the
/// status the thread should currently show. Sorts by `received_at`
/// ascending internally so callers can hand in DESC-ordered store
/// results without flipping them first.
pub fn derive_thread_status(events: &[HookEvent]) -> AgentStatusState {
    let mut sorted: Vec<&HookEvent> = events.iter().collect();
    sorted.sort_by_key(|e| e.received_at);

    let mut state = AgentStatusState::Idle;
    // Subagent-in-flight count: while a `Task` tool dispatched by the
    // parent is still running, a `Stop` event for the parent must NOT
    // flip status back to waiting — the parent is genuinely still
    // working. Mirrors main's `pendingTasks`.
    let mut pending_tasks: i32 = 0;
    // ExitPlanMode is Claude Code's built-in plan-approval prompt.
    // PreToolUse fires when the agent invokes it; PostToolUse only
    // arrives once the user approves or rejects. While that gap is
    // open the agent is genuinely waiting on the user, so override
    // the derived state at the end.
    let mut pending_exit_plan_mode: i32 = 0;

    for ev in sorted {
        match ev.kind {
            HookKind::UserPromptSubmit => {
                state = AgentStatusState::Running;
            }
            HookKind::PreToolUse => {
                state = AgentStatusState::Running;
                match payload_tool_name(&ev.payload_json).as_deref() {
                    Some("Task") => pending_tasks += 1,
                    Some("ExitPlanMode") => pending_exit_plan_mode += 1,
                    _ => {}
                }
            }
            HookKind::PostToolUse => {
                state = AgentStatusState::Running;
                match payload_tool_name(&ev.payload_json).as_deref() {
                    Some("Task") if pending_tasks > 0 => pending_tasks -= 1,
                    Some("ExitPlanMode") if pending_exit_plan_mode > 0 => {
                        pending_exit_plan_mode -= 1;
                    }
                    _ => {}
                }
            }
            HookKind::Stop => {
                state = if pending_tasks > 0 {
                    AgentStatusState::Running
                } else {
                    AgentStatusState::Idle
                };
            }
            HookKind::SubagentStop => {
                // SubagentStop itself doesn't change the parent's
                // status; the matching PostToolUse for the Task tool
                // is what decrements pending_tasks. Defensive
                // decrement anyway so a missing PostToolUse doesn't
                // strand the count and pin status to working forever.
                if pending_tasks > 0 {
                    pending_tasks -= 1;
                }
            }
            HookKind::Interrupt => {
                state = AgentStatusState::Idle;
                pending_tasks = 0;
                pending_exit_plan_mode = 0;
            }
            HookKind::AgentBoot => {
                state = AgentStatusState::Idle;
                pending_tasks = 0;
                pending_exit_plan_mode = 0;
            }
        }
    }

    if pending_exit_plan_mode > 0 {
        AgentStatusState::AwaitingUser
    } else {
        state
    }
}

fn payload_tool_name(payload: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    v.get("tool_name")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::{HookEventId, ThreadId, Timestamp};

    fn ev(kind: HookKind, ms: i64, payload: &str) -> HookEvent {
        HookEvent {
            id: HookEventId::new(ms),
            thread_id: Some(ThreadId::new(1)),
            stream_id: None,
            kind,
            session_id: None,
            payload_json: payload.to_string(),
            received_at: Timestamp::from_unix_ms(ms),
        }
    }

    #[test]
    fn empty_log_is_idle() {
        assert_eq!(derive_thread_status(&[]), AgentStatusState::Idle);
    }

    #[test]
    fn user_prompt_then_stop_idles() {
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::Stop, 2, "{}"),
        ];
        assert_eq!(derive_thread_status(&events), AgentStatusState::Idle);
    }

    #[test]
    fn user_prompt_running_then_tool_use_keeps_running() {
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Edit"}"#),
        ];
        assert_eq!(derive_thread_status(&events), AgentStatusState::Running);
    }

    #[test]
    fn task_dispatch_keeps_running_through_stop() {
        // Parent dispatches Task subagent, then a Stop fires before
        // the subagent's PostToolUse. Parent is still working.
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Task"}"#),
            ev(HookKind::Stop, 3, "{}"),
        ];
        assert_eq!(derive_thread_status(&events), AgentStatusState::Running);
    }

    #[test]
    fn task_completes_then_stop_idles() {
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Task"}"#),
            ev(HookKind::PostToolUse, 3, r#"{"tool_name":"Task"}"#),
            ev(HookKind::Stop, 4, "{}"),
        ];
        assert_eq!(derive_thread_status(&events), AgentStatusState::Idle);
    }

    #[test]
    fn unsorted_input_is_normalized() {
        // Same events as the prior test but handed in DESC order.
        let events = [
            ev(HookKind::Stop, 4, "{}"),
            ev(HookKind::PostToolUse, 3, r#"{"tool_name":"Task"}"#),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Task"}"#),
            ev(HookKind::UserPromptSubmit, 1, "{}"),
        ];
        assert_eq!(derive_thread_status(&events), AgentStatusState::Idle);
    }

    #[test]
    fn interrupt_drops_to_idle_and_clears_pending() {
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Task"}"#),
            ev(HookKind::Interrupt, 3, "{}"),
        ];
        assert_eq!(derive_thread_status(&events), AgentStatusState::Idle);
    }

    #[test]
    fn exit_plan_mode_pending_shows_awaiting_user() {
        // Claude Code's built-in plan-mode approval: PreToolUse fires
        // when the agent calls ExitPlanMode, but the matching
        // PostToolUse only arrives once the user approves. Until
        // then, we are waiting on the user, not working.
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"ExitPlanMode"}"#),
        ];
        assert_eq!(
            derive_thread_status(&events),
            AgentStatusState::AwaitingUser
        );
    }

    #[test]
    fn exit_plan_mode_completed_no_longer_awaiting_user() {
        // After PostToolUse(ExitPlanMode), the user has answered;
        // status falls back to whatever the last hook implies.
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"ExitPlanMode"}"#),
            ev(HookKind::PostToolUse, 3, r#"{"tool_name":"ExitPlanMode"}"#),
        ];
        assert_eq!(derive_thread_status(&events), AgentStatusState::Running);
    }

    #[test]
    fn subagent_stop_decrements_pending_when_post_tool_use_missing() {
        // Defensive: if SubagentStop arrives without a matching
        // Task PostToolUse, decrement so the parent's Stop can idle
        // out instead of being stuck at Running forever.
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Task"}"#),
            ev(HookKind::SubagentStop, 3, "{}"),
            ev(HookKind::Stop, 4, "{}"),
        ];
        assert_eq!(derive_thread_status(&events), AgentStatusState::Idle);
    }
}
