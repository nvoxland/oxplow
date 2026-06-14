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

use oxplow_domain::{AgentStatusState, HookEvent, HookKind, Timestamp};

/// How long a `Running` thread may go without emitting any hook event
/// before the derivation declares it `Stalled`. Claude Code emits no
/// hook when a turn dies on an API error (socket closed, etc.) and the
/// process drops back to its prompt — the log just stops mid-Running.
/// An active turn emits Pre/PostToolUse continuously; the longest
/// silent stretch is a single max-timeout Bash call (10 minutes), so
/// 15 minutes clears that with margin while still catching real
/// deaths quickly.
pub const AGENT_STALL_AFTER_MS: i64 = 15 * 60 * 1000;

/// Shorter death threshold for the case where the agent is NOT inside
/// an open tool call (tsk130). When a turn dies between steps — a
/// model-unavailable error ("Claude Fable 5 is currently unavailable")
/// or a transient API death right after a prompt or between tool calls
/// — Claude Code emits no hook and the log goes silent with no
/// PreToolUse left open. There's no long-running Bash to wait out, so
/// the only reason for silence is death: catch it in 5 minutes instead
/// of the full [`AGENT_STALL_AFTER_MS`]. The longer threshold still
/// governs the open-tool case (a single Bash can legitimately run up to
/// its 10-minute max with no intervening hook).
pub const AGENT_DEAD_AFTER_MS: i64 = 5 * 60 * 1000;

/// Replay `events` (which may arrive in any order) and return the
/// status the thread should currently show *as of `now`*. Sorts by
/// `received_at` ascending internally so callers can hand in
/// DESC-ordered store results without flipping them first.
///
/// Time-awareness: a derived `Running` whose newest event is older than
/// its silence threshold degrades to `Stalled` — the agent process
/// almost certainly died (or errored back to its prompt) without
/// emitting a Stop hook. The threshold depends on whether a tool call
/// is still open: [`AGENT_STALL_AFTER_MS`] when one is (a long Bash can
/// run silently up to its max), the shorter [`AGENT_DEAD_AFTER_MS`]
/// when nothing is open (silence between steps means death, caught
/// promptly — tsk130). `AwaitingUser` is exempt: waiting on the user
/// indefinitely is legitimate.
///
/// This is the hook-only entry point — it has no view of PTY output,
/// so a single long turn that emits no hooks between tool calls (just
/// streaming tokens) would wrongly degrade to `Stalled`. Callers that
/// can observe terminal liveness should prefer
/// [`derive_thread_status_with_activity`].
pub fn derive_thread_status(events: &[HookEvent], now: Timestamp) -> AgentStatusState {
    derive_thread_status_with_activity(events, None, now)
}

/// Like [`derive_thread_status`], but folds in the thread's most recent
/// PTY output timestamp (`last_output_at`) when deciding whether a
/// `Running` turn has gone silent.
///
/// Hooks are sparse *within* a turn: a single long turn streams tokens
/// to the terminal for many minutes while emitting no Pre/PostToolUse
/// between tool calls, so a frozen hook log alone reads as death even
/// though the agent is plainly working (tsk141). Output bytes are the
/// missing cadence signal — an agent still writing to its PTY is alive
/// regardless of how stale its last hook is.
///
/// The stall decision therefore measures silence from the *later* of
/// the newest hook event and `last_output_at`. Genuine death stays
/// detected because a dead turn stops emitting output too: once both
/// signals are quiet past the threshold (short or long per the
/// open-tool rule), the turn degrades to `Stalled` exactly as before
/// (tsk130 intact). `last_output_at = None` reproduces the old
/// hook-only behavior.
pub fn derive_thread_status_with_activity(
    events: &[HookEvent],
    last_output_at: Option<Timestamp>,
    now: Timestamp,
) -> AgentStatusState {
    let mut sorted: Vec<&HookEvent> = events.iter().collect();
    sorted.sort_by_key(|e| e.received_at);

    let mut state = AgentStatusState::Idle;
    // Subagent-in-flight count: while a `Task` tool dispatched by the
    // parent is still running, a `Stop` event for the parent must NOT
    // flip status back to waiting — the parent is genuinely still
    // working. Mirrors main's `pendingTasks`.
    let mut pending_tasks: i32 = 0;
    // User-input tools: PreToolUse fires when the agent asks the user
    // something; the matching PostToolUse only arrives once the user
    // answers. While that gap is open the agent is genuinely waiting on
    // the user (no Stop hook fires either), so override the derived
    // state at the end and exempt it from the stall threshold. Two such
    // tools exist: ExitPlanMode (the built-in plan-approval prompt) and
    // AskUserQuestion (the built-in clarifying-question prompt). Before
    // tsk128 only ExitPlanMode was tracked, so an agent blocked on
    // AskUserQuestion stayed Running and degraded to Stalled — read as
    // a death and mis-triggering a re-dispatch.
    let mut pending_user_input: i32 = 0;
    // Count of currently-open tool calls (any PreToolUse without its
    // matching PostToolUse). Distinguishes "the agent is inside a tool
    // that may legitimately run long (a 10-min Bash)" from "the agent
    // is between steps with nothing running" — the two get different
    // silence thresholds below (tsk130).
    let mut open_tools: i32 = 0;

    for ev in &sorted {
        match ev.kind {
            HookKind::UserPromptSubmit => {
                state = AgentStatusState::Running;
            }
            HookKind::PreToolUse => {
                state = AgentStatusState::Running;
                open_tools += 1;
                match payload_tool_name(&ev.payload_json).as_deref() {
                    Some("Task") => pending_tasks += 1,
                    Some(t) if is_user_input_tool(t) => pending_user_input += 1,
                    _ => {}
                }
            }
            HookKind::PostToolUse => {
                state = AgentStatusState::Running;
                if open_tools > 0 {
                    open_tools -= 1;
                }
                match payload_tool_name(&ev.payload_json).as_deref() {
                    Some("Task") if pending_tasks > 0 => pending_tasks -= 1,
                    Some(t) if is_user_input_tool(t) && pending_user_input > 0 => {
                        pending_user_input -= 1;
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
                pending_user_input = 0;
                open_tools = 0;
            }
            HookKind::AgentBoot => {
                state = AgentStatusState::Idle;
                pending_tasks = 0;
                pending_user_input = 0;
                open_tools = 0;
            }
        }
    }

    if pending_user_input > 0 {
        return AgentStatusState::AwaitingUser;
    }
    if state == AgentStatusState::Running {
        if let Some(last) = sorted.last() {
            // An open tool call (e.g. a long Bash) may legitimately run
            // silently up to its max; a turn with nothing open that goes
            // silent has almost certainly died between steps. Pick the
            // threshold accordingly (tsk130).
            let threshold = if open_tools > 0 {
                AGENT_STALL_AFTER_MS
            } else {
                AGENT_DEAD_AFTER_MS
            };
            // Silence is measured from the LATER of the last hook and
            // the last PTY output: a long turn streaming tokens with no
            // intervening hook is alive (tsk141), while a genuinely-dead
            // turn goes quiet on both signals and still degrades (tsk130).
            let last_activity_ms = last
                .received_at
                .unix_ms()
                .max(last_output_at.map(|t| t.unix_ms()).unwrap_or(i64::MIN));
            if now.unix_ms() - last_activity_ms > threshold {
                return AgentStatusState::Stalled;
            }
        }
    }
    state
}

/// Built-in tools that block the turn waiting on a human answer. A
/// PreToolUse for one of these with no matching PostToolUse means the
/// agent is parked on the user, not working and not dead.
fn is_user_input_tool(tool_name: &str) -> bool {
    matches!(tool_name, "ExitPlanMode" | "AskUserQuestion")
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

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_unix_ms(ms)
    }

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
        assert_eq!(derive_thread_status(&[], at(10)), AgentStatusState::Idle);
    }

    #[test]
    fn user_prompt_then_stop_idles() {
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::Stop, 2, "{}"),
        ];
        assert_eq!(
            derive_thread_status(&events, at(10)),
            AgentStatusState::Idle
        );
    }

    #[test]
    fn user_prompt_running_then_tool_use_keeps_running() {
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Edit"}"#),
        ];
        assert_eq!(
            derive_thread_status(&events, at(10)),
            AgentStatusState::Running
        );
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
        assert_eq!(
            derive_thread_status(&events, at(10)),
            AgentStatusState::Running
        );
    }

    #[test]
    fn task_completes_then_stop_idles() {
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Task"}"#),
            ev(HookKind::PostToolUse, 3, r#"{"tool_name":"Task"}"#),
            ev(HookKind::Stop, 4, "{}"),
        ];
        assert_eq!(
            derive_thread_status(&events, at(10)),
            AgentStatusState::Idle
        );
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
        assert_eq!(
            derive_thread_status(&events, at(10)),
            AgentStatusState::Idle
        );
    }

    #[test]
    fn interrupt_drops_to_idle_and_clears_pending() {
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Task"}"#),
            ev(HookKind::Interrupt, 3, "{}"),
        ];
        assert_eq!(
            derive_thread_status(&events, at(10)),
            AgentStatusState::Idle
        );
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
            derive_thread_status(&events, at(10)),
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
        assert_eq!(
            derive_thread_status(&events, at(10)),
            AgentStatusState::Running
        );
    }

    #[test]
    fn running_with_stale_log_degrades_to_stalled() {
        // The API-error death: the log ends mid-Running (no Stop ever
        // arrives) and wall-clock time keeps moving. Past the stall
        // threshold the derivation must stop claiming Running.
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Edit"}"#),
        ];
        assert_eq!(
            derive_thread_status(&events, at(2 + AGENT_STALL_AFTER_MS + 1)),
            AgentStatusState::Stalled
        );
    }

    #[test]
    fn running_within_threshold_stays_running() {
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Edit"}"#),
        ];
        assert_eq!(
            derive_thread_status(&events, at(2 + AGENT_STALL_AFTER_MS)),
            AgentStatusState::Running
        );
    }

    #[test]
    fn dead_with_no_open_tool_stalls_at_short_threshold() {
        // tsk130: the genuinely-dead turn. A model-unavailable / API
        // death right after the prompt leaves the log silent with NO
        // open tool call (nothing to wait out). It must surface as
        // Stalled at the SHORT threshold, well before the 15-min stall
        // window — the user once watched this sit "Working" for ~1h.
        let events = [ev(HookKind::UserPromptSubmit, 1, "{}")];
        assert_eq!(
            derive_thread_status(&events, at(1 + AGENT_DEAD_AFTER_MS + 1)),
            AgentStatusState::Stalled
        );
    }

    #[test]
    fn dead_between_tool_calls_stalls_at_short_threshold() {
        // Death between steps: the last tool returned (Pre+Post pair,
        // nothing open), then the next model call died. No open tool
        // to wait out → short threshold applies.
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Edit"}"#),
            ev(HookKind::PostToolUse, 3, r#"{"tool_name":"Edit"}"#),
        ];
        assert_eq!(
            derive_thread_status(&events, at(3 + AGENT_DEAD_AFTER_MS + 1)),
            AgentStatusState::Stalled
        );
    }

    #[test]
    fn no_open_tool_within_short_threshold_stays_running() {
        // Just below the short threshold the agent is presumed alive
        // (the model is composing the next step).
        let events = [ev(HookKind::UserPromptSubmit, 1, "{}")];
        assert_eq!(
            derive_thread_status(&events, at(1 + AGENT_DEAD_AFTER_MS)),
            AgentStatusState::Running
        );
    }

    #[test]
    fn open_tool_call_uses_long_threshold_not_short() {
        // A single Bash can legitimately run up to its 10-min max with
        // no intervening hook. While a tool call is OPEN (PreToolUse
        // with no matching PostToolUse), silence past the SHORT
        // threshold must NOT be read as death — only the long stall
        // window applies.
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Bash"}"#),
        ];
        // Past the short death threshold, but under the long stall one.
        assert_eq!(
            derive_thread_status(&events, at(2 + AGENT_DEAD_AFTER_MS + 1)),
            AgentStatusState::Running
        );
        // Past the long stall threshold it finally degrades.
        assert_eq!(
            derive_thread_status(&events, at(2 + AGENT_STALL_AFTER_MS + 1)),
            AgentStatusState::Stalled
        );
    }

    #[test]
    fn stale_idle_log_is_not_stalled() {
        // Only Running degrades — a thread that stopped cleanly hours
        // ago is just idle, not stalled.
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::Stop, 2, "{}"),
        ];
        assert_eq!(
            derive_thread_status(&events, at(2 + AGENT_STALL_AFTER_MS * 10)),
            AgentStatusState::Idle
        );
    }

    #[test]
    fn stale_exit_plan_mode_stays_awaiting_user() {
        // Waiting on the user indefinitely is legitimate — the plan
        // approval gap must not degrade to Stalled.
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"ExitPlanMode"}"#),
        ];
        assert_eq!(
            derive_thread_status(&events, at(2 + AGENT_STALL_AFTER_MS * 10)),
            AgentStatusState::AwaitingUser
        );
    }

    #[test]
    fn ask_user_question_pending_shows_awaiting_user() {
        // tsk128: the dogfooding bug. AskUserQuestion is a built-in
        // tool whose PreToolUse fires when the agent asks the user a
        // question; the matching PostToolUse only lands once the user
        // answers. Until then the agent is WAITING ON THE USER — it
        // must surface AwaitingUser, never Running (and never Stalled).
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(
                HookKind::PreToolUse,
                2,
                r#"{"tool_name":"AskUserQuestion"}"#,
            ),
        ];
        assert_eq!(
            derive_thread_status(&events, at(10)),
            AgentStatusState::AwaitingUser
        );
    }

    #[test]
    fn stale_ask_user_question_stays_awaiting_user_not_stalled() {
        // The crux of the bug: two agents blocked on AskUserQuestion
        // were shown as "Stalled — agent stopped responding mid-turn"
        // and misread as dead. Waiting on the user indefinitely is
        // legitimate, so the stall threshold must NOT degrade it.
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(
                HookKind::PreToolUse,
                2,
                r#"{"tool_name":"AskUserQuestion"}"#,
            ),
        ];
        assert_eq!(
            derive_thread_status(&events, at(2 + AGENT_STALL_AFTER_MS * 10)),
            AgentStatusState::AwaitingUser
        );
    }

    #[test]
    fn ask_user_question_answered_no_longer_awaiting_user() {
        // After PostToolUse(AskUserQuestion) the user has answered;
        // status falls back to Running like any other completed tool.
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(
                HookKind::PreToolUse,
                2,
                r#"{"tool_name":"AskUserQuestion"}"#,
            ),
            ev(
                HookKind::PostToolUse,
                3,
                r#"{"tool_name":"AskUserQuestion"}"#,
            ),
        ];
        assert_eq!(
            derive_thread_status(&events, at(10)),
            AgentStatusState::Running
        );
    }

    #[test]
    fn long_turn_with_ongoing_output_is_working_not_stalled() {
        // tsk141: a single ~1h turn streams tokens to the terminal but
        // emits NO hook between tool calls. The hook log is frozen at
        // turn start (well past the short death threshold with nothing
        // open), yet the PTY is still advancing — that's Working, not
        // Stalled. A frozen updated_at alone must not flip it.
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Edit"}"#),
            ev(HookKind::PostToolUse, 3, r#"{"tool_name":"Edit"}"#),
        ];
        let now = at(3 + AGENT_STALL_AFTER_MS + 1);
        // Hook-only view (no output): would wrongly read as Stalled.
        assert_eq!(
            derive_thread_status(&events, now),
            AgentStatusState::Stalled
        );
        // With output advancing right up to `now`, it stays Running.
        let last_output = at(now.unix_ms() - 1000);
        assert_eq!(
            derive_thread_status_with_activity(&events, Some(last_output), now),
            AgentStatusState::Running
        );
    }

    #[test]
    fn quiescent_output_past_threshold_flips_to_stalled() {
        // The inverse: output WAS flowing but has now gone quiet past
        // the threshold (and so has the hook log). With nothing open,
        // the short death threshold applies — degrade to Stalled.
        let events = [
            ev(HookKind::UserPromptSubmit, 1, "{}"),
            ev(HookKind::PreToolUse, 2, r#"{"tool_name":"Edit"}"#),
            ev(HookKind::PostToolUse, 3, r#"{"tool_name":"Edit"}"#),
        ];
        let last_output = at(100);
        let now = at(last_output.unix_ms() + AGENT_DEAD_AFTER_MS + 1);
        assert_eq!(
            derive_thread_status_with_activity(&events, Some(last_output), now),
            AgentStatusState::Stalled
        );
    }

    #[test]
    fn recent_output_uses_short_threshold_window_correctly() {
        // Output within the short window keeps a no-open-tool turn alive
        // even though the last hook is older than the death threshold.
        let events = [ev(HookKind::UserPromptSubmit, 1, "{}")];
        let last_output = at(1 + AGENT_DEAD_AFTER_MS); // hook is stale by now
        let now = at(last_output.unix_ms() + AGENT_DEAD_AFTER_MS); // exactly at threshold
        assert_eq!(
            derive_thread_status_with_activity(&events, Some(last_output), now),
            AgentStatusState::Running
        );
    }

    #[test]
    fn stale_output_does_not_revive_a_dead_turn() {
        // Genuine-death guard (tsk130 intact): if the last output is
        // ALSO older than the threshold, the activity signal can't mask
        // the death — it still degrades to Stalled.
        let events = [ev(HookKind::UserPromptSubmit, 1, "{}")];
        let stale_output = at(50);
        let now = at(stale_output.unix_ms() + AGENT_DEAD_AFTER_MS + 1);
        assert_eq!(
            derive_thread_status_with_activity(&events, Some(stale_output), now),
            AgentStatusState::Stalled
        );
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
        assert_eq!(
            derive_thread_status(&events, at(10)),
            AgentStatusState::Idle
        );
    }
}
