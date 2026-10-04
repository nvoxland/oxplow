//! The policy every agent transport enforces (tsk333): the write guard,
//! filing enforcement and the end-of-turn Stop directive, with the I/O
//! they need (the thread, the stream's claim, git state, pending effort
//! reviews). Decisions are transport-neutral: the hook route renders them
//! as Claude's JSON, and the ACP client as permission answers and a
//! banner the human sees. The pure rules live in `oxplow_runtime`
//! (`policy`, `stop_hook`). See `.context/agent-model.md`.

use std::collections::HashMap;
use std::path::Path;

use oxplow_domain::stores::{TaskStore, ThreadStore};
use oxplow_domain::{Thread, ThreadId};
use oxplow_runtime::policy::{decide_tool, IntentKind, PolicyDecision, PolicyFacts, ToolIntent};
use oxplow_runtime::stop_hook::{
    decide_stop_directive, DirectiveBuilders, OpenEffort, PendingEffortReview, StopDirective,
    StopHookSideEffect, ThreadSnapshot,
};
use parking_lot::Mutex;

use crate::Services;

/// In-memory state the Stop pipeline keeps across turns. Runtime-only:
/// losing it on a daemon restart costs at most one repeated audit.
#[derive(Default)]
struct StopState {
    /// Last in-progress audit signature emitted per thread; dedupes
    /// back-to-back audits while the in_progress set hasn't changed.
    last_audit_signature: HashMap<ThreadId, String>,
    /// Threads where the "filed-but-didn't-ship" advisory already fired.
    filed_but_didnt_ship_fired: HashMap<ThreadId, bool>,
}

/// The shared agent policy (`Services.agent_policy`).
#[derive(Default)]
pub struct AgentPolicy {
    stop_state: Mutex<StopState>,
}

/// A Claude-shaped tool call (the hook payload, or an opencode call its
/// bridge mapped onto Claude's names) as a policy intent.
#[derive(Debug, Clone)]
pub struct ClaudeIntent {
    pub label: String,
    pub kind: IntentKind,
    pub paths: Vec<String>,
}

/// Map a Claude-shaped `{tool_name, tool_input}` to an intent. `None` for a
/// tool neither rule can refuse (Read, Grep, Bash, MCP, Task, …), so
/// callers skip the policy's I/O entirely.
pub fn claude_intent(body: &serde_json::Value) -> Option<ClaudeIntent> {
    use oxplow_runtime::filing::ALWAYS_WRITE_INTENT_TOOL_NAMES;
    use oxplow_runtime::write_guard::WORKTREE_MUTATING_TOOLS;
    let tool_name = body.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
    if !(WORKTREE_MUTATING_TOOLS.contains(&tool_name)
        || ALWAYS_WRITE_INTENT_TOOL_NAMES.contains(&tool_name))
    {
        return None;
    }
    // The first key holding a string: a null `file_path` mustn't hide a
    // `notebook_path` (tsk371).
    let path = body.get("tool_input").and_then(|t| {
        ["file_path", "notebook_path", "path"]
            .iter()
            .find_map(|k| t.get(k).and_then(|v| v.as_str()))
    });
    Some(ClaudeIntent {
        label: tool_name.to_string(),
        kind: IntentKind::WorktreeWrite,
        paths: path.map(|p| vec![p.to_string()]).unwrap_or_default(),
    })
}

impl AgentPolicy {
    /// May an agent run `spec`? The bus has already admitted the invoker;
    /// this is the agent-specific layer on top: a `Write` command needs a
    /// thread that may write (`may_write`, from the bus's write gate —
    /// a queued or closed thread can read, not change). `None` means the
    /// gate had nothing to say (a read, or no thread/gate).
    pub fn check_command(
        &self,
        thread_id: Option<&ThreadId>,
        spec: &oxplow_domain::CommandSpec,
        may_write: Option<bool>,
    ) -> PolicyDecision {
        if !spec.invokers.agent {
            return PolicyDecision::Deny {
                layer: oxplow_runtime::policy::DenyLayer::Command,
                reason: format!("`{}` is not open to agents", spec.name),
            };
        }
        if may_write == Some(false) {
            return PolicyDecision::Deny {
                layer: oxplow_runtime::policy::DenyLayer::Command,
                reason: format!(
                    "`{}` changes state, and thread {} may not write (only the stream's writer thread can)",
                    spec.name,
                    thread_id.map(|t| t.to_string()).unwrap_or_default()
                ),
            };
        }
        PolicyDecision::Allow
    }

    /// May `thread_id` run `intent` now? Allows when the thread is
    /// unknown (nothing to enforce against).
    pub async fn check_tool(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        intent: &ToolIntent<'_>,
    ) -> PolicyDecision {
        if intent.kind != IntentKind::WorktreeWrite {
            return PolicyDecision::Allow;
        }
        let Some(thread) = svc.thread_store.get(thread_id).await.ok().flatten() else {
            return PolicyDecision::Allow;
        };
        let project_dir = svc.layout.project_dir.as_path();
        // The thread's own tree is its stream's worktree (a sibling dir for
        // a worktree stream); every other stream's tree is off limits.
        let streams = oxplow_domain::stores::StreamStore::list(svc.stream_store.as_ref())
            .await
            .unwrap_or_default();
        let worktree_root = svc.thread_worktree(thread_id).await;
        let other_roots: Vec<std::path::PathBuf> = streams
            .iter()
            .filter(|s| s.id != thread.stream_id && !s.worktree_path.is_empty())
            .map(|s| std::path::PathBuf::from(&s.worktree_path))
            .chain(std::iter::once(project_dir.to_path_buf()))
            .filter(|p| p != &worktree_root)
            .collect();
        // Only the writer is subject to filing; skip its lookups otherwise.
        let (has_open_effort, git_operation_in_progress) = if thread.status.is_writer() {
            (
                stream_has_open_effort(svc, &thread).await,
                git_operation_in_progress(&worktree_root),
            )
        } else {
            (false, false)
        };
        let active_work_items = svc.work_items.active();
        decide_tool(
            intent,
            &PolicyFacts {
                thread: &thread,
                worktree_root: &worktree_root,
                other_roots: &other_roots,
                project_dir,
                has_open_effort,
                git_operation_in_progress,
                active_work_items: &active_work_items,
            },
        )
    }

    /// The Stop directive for the writer thread at the end of a turn. Pulls the current
    /// in_progress set, runs `decide_stop_directive` with the in-memory
    /// audit-signature dedup, and persists the side effects back to
    /// `StopState`.
    pub async fn on_turn_end(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        turn_signals: Option<&TurnSignals>,
    ) -> Option<StopDirective> {
        use oxplow_db::EffortStore as _;
        let thread = svc.thread_store.get(thread_id).await.ok().flatten()?;

        let tasks = svc
            .task_store
            .list_for_thread(thread_id)
            .await
            .ok()
            .unwrap_or_default();

        let last_signature = self
            .stop_state
            .lock()
            .last_audit_signature
            .get(thread_id)
            .cloned();
        let filed_but_didnt_ship_fired = self
            .stop_state
            .lock()
            .filed_but_didnt_ship_fired
            .get(thread_id)
            .copied()
            .unwrap_or(false);

        // Drain any pending effort review ids `effort.report`
        // handler stashed for this thread. For each, recompute the
        // review against the live `effort_file` rows so an agent
        // that already amended doesn't get a stale prompt. Drop the ones
        // that no longer carry a discrepancy. Title resolution joins
        // each effort's task title for the directive text.
        let pending_ids = svc.thread_runtime.take_pending_effort_reviews(thread_id);
        let mut pending_reviews: Vec<PendingEffortReview> = Vec::new();
        if !pending_ids.is_empty() {
            let titles_by_id: std::collections::HashMap<i64, String> = tasks
                .iter()
                .map(|t| (t.id.value(), t.title.clone()))
                .collect();
            for eid in pending_ids {
                // Two reconcilable kinds share this surface: files (recomputed
                // against live `effort_file` rows) and test runs (the
                // `effort_attribution` ledger's unattributed residue). An effort
                // is worth surfacing if EITHER still carries something to triage.
                let file_review = crate::task_service::recompute_effort_file_review(
                    &svc.effort_store,
                    &svc.snapshot_store,
                    &eid,
                )
                .await;
                let unattributed_refs = svc
                    .attribution_store
                    .list_refs(&eid, "run", oxplow_db::STATE_UNATTRIBUTED)
                    .await
                    .unwrap_or_default();
                if file_review.is_none() && unattributed_refs.is_empty() {
                    continue;
                }
                // Enrich each bare `run:<id>` ref into a human descriptor the agent
                // can actually recognize ("run:47 — cargo test (419 passed, 0
                // failed) @ 10:03") — the ref stays at the front so `claim_runs`
                // still parses it (tsk266).
                let mut unattributed_runs = Vec::with_capacity(unattributed_refs.len());
                for r in &unattributed_refs {
                    unattributed_runs.push(describe_run(&svc.fact_store, r).await);
                }
                // The work item comes from the file review when present;
                // otherwise from the effort (run-only residue, no file
                // discrepancy). An oxplow task shows its title.
                let work_item = match file_review.as_ref() {
                    Some(r) => r.work_item.clone(),
                    None => svc
                        .effort_store
                        .work_item_for_effort(&eid)
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or_default(),
                };
                // The effort row can be gone (its thread was deleted); the
                // review then names the effort alone.
                let title = if work_item.is_empty() {
                    format!("effort {eid}")
                } else {
                    oxplow_domain::refs::build::task_of_work_item_ref(&work_item)
                        .and_then(|t| titles_by_id.get(&t.value()).cloned())
                        .unwrap_or_else(|| oxplow_domain::refs::build::work_item_label(&work_item))
                };
                pending_reviews.push(PendingEffortReview {
                    effort_id: file_review
                        .as_ref()
                        .map(|r| r.effort_id.clone())
                        .unwrap_or_else(|| eid.to_string()),
                    work_item,
                    task_title: title,
                    claimed_but_not_changed: file_review
                        .as_ref()
                        .map(|r| r.claimed_but_not_changed.clone())
                        .unwrap_or_default(),
                    changed_but_not_claimed: file_review
                        .as_ref()
                        .map(|r| r.changed_but_not_claimed.clone())
                        .unwrap_or_default(),
                    unclaimed_overflow: file_review.as_ref().and_then(|r| r.unclaimed_overflow),
                    unattributed_runs,
                });
            }
        }

        let open_efforts = open_efforts_in_stream(svc, &thread).await;
        let snapshot = ThreadSnapshot {
            thread: Some(&thread),
            tasks: &tasks,
            open_efforts: &open_efforts,
            last_in_progress_audit_signature: last_signature.as_deref(),
            // Read from the events anchored to the closed turn
            // (`TurnSignals::of_turn`). Letting the Q&A-turn carve-out fire
            // silences the audit nudge on read-only / one-off question
            // turns where there's no work to claim.
            turn_had_activity: turn_signals.map(|s| s.had_activity),
            turn_had_writes: turn_signals.map(|s| s.had_writes).unwrap_or(false),
            subagent_in_flight: turn_signals.is_some_and(|s| s.subagent_in_flight),
            awaiting_user: turn_signals.is_some_and(|s| s.awaiting_user),
            // Not yet wired (default false ⇒ the branch stays silent rather
            // than emit a wrong directive): turn_had_filing /
            // turn_filed_ready_item need MCP call attribution to the turn.
            turn_had_filing: false,
            turn_filed_ready_item: false,
            filed_but_didnt_ship_fired,
            pending_effort_reviews: &pending_reviews,
        };

        let outcome = decide_stop_directive(
            snapshot,
            DirectiveBuilders {
                build_open_effort_audit_reason: Some(&build_open_effort_audit_reason),
                build_filed_but_didnt_ship_reason: Some(&build_filed_but_didnt_ship_reason),
                build_stale_epic_children_reason: None,
                build_effort_file_review_reason: Some(&build_effort_file_review_reason),
            },
        );

        // Apply side effects to the in-memory state.
        {
            let mut st = self.stop_state.lock();
            for eff in &outcome.side_effects {
                match eff {
                    StopHookSideEffect::RecordAuditSignature(sig) => {
                        st.last_audit_signature.insert(*thread_id, sig.clone());
                    }
                    StopHookSideEffect::RecordFiledButDidntShipFired => {
                        st.filed_but_didnt_ship_fired.insert(*thread_id, true);
                    }
                }
            }
        }

        outcome.directive
    }
}

/// What a turn did, from the tool events anchored to it (P3.4). Powers
/// the Stop pipeline's Q&A-turn carve-out and the writes-vs-no-writes
/// branch of the filed-but-didn't-ship advisory.
#[derive(Debug, Clone, Default)]
pub struct TurnSignals {
    /// The turn requested or finished at least one tool.
    pub had_activity: bool,
    /// One of them was Edit/Write/MultiEdit/NotebookEdit (a refused write
    /// still counts: the agent tried to change files).
    pub had_writes: bool,
    /// The turn parked on the person: an `agent.status.changed
    /// {awaiting_user}` anchored to it (`await_user`, or the Stop's own).
    pub awaiting_user: bool,
    /// A subagent it started (an allowed `Task` request) hadn't finished
    /// when the turn ended.
    pub subagent_in_flight: bool,
}

/// The tool names that start a subagent. One list, shared with the status
/// derivation.
pub const SUBAGENT_TOOLS: &[&str] = &["Task", "Agent"];

impl TurnSignals {
    /// The signals of `turn`, read from the tool and status events
    /// anchored to it — exact by turn, so neither a turn before nor one
    /// alongside can leak in.
    pub async fn of_turn(
        db: &oxplow_db::Database,
        turn: oxplow_domain::AgentTurnId,
    ) -> Result<Self, oxplow_domain::DomainError> {
        let subagent = SUBAGENT_TOOLS
            .iter()
            .map(|t| format!("'{t}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT coalesce(sum(type <> 'agent.status.changed'), 0) > 0,
                    coalesce(max(type <> 'agent.status.changed'
                                 AND json_extract(payload, '$.tool')
                                     IN ('Edit', 'Write', 'MultiEdit', 'NotebookEdit')), 0),
                    coalesce(max(type = 'agent.status.changed'
                                 AND json_extract(payload, '$.state') = 'awaiting_user'), 0),
                    coalesce(sum(type = 'agent.tool.requested'
                                 AND json_extract(payload, '$.decision') = 'allowed'
                                 AND json_extract(payload, '$.tool') IN ({subagent})), 0)
                  > coalesce(sum(type = 'agent.tool.finished'
                                 AND json_extract(payload, '$.tool') IN ({subagent})), 0)
               FROM event_log
              WHERE turn_id = ?1
                AND type IN ('agent.tool.requested', 'agent.tool.finished',
                             'agent.status.changed')
                AND payload_expired_at IS NULL"
        );
        db.read(move |conn| {
            conn.query_row(&sql, [turn.value()], |r| {
                Ok(TurnSignals {
                    had_activity: r.get(0)?,
                    had_writes: r.get(1)?,
                    awaiting_user: r.get(2)?,
                    subagent_in_flight: r.get(3)?,
                })
            })
            .map_err(oxplow_db::map_sql_err)
        })
        .await
    }
}

/// Whether the writer's stream has an open effort — the filing guard's
/// claim (P2.7, tsk431). An effort opens when a task is filed or moved
/// `in_progress` (in the same transaction) or through `effort.open` for
/// another provider's work item; an `in_progress` row alone isn't one.
///
/// Scoped to the whole STREAM, not just the literal thread (tsk133): a
/// stream has exactly one active writer (the `idx_threads_one_active_per_stream`
/// unique index + the write guard), so an effort on *any* of its threads
/// is a legitimate claim for the writer — cross-thread dispatch needs no
/// `move_task` first. Queued/closed threads still can't write (the write
/// guard runs first). A lookup failure denies (fails closed).
pub(crate) async fn stream_has_open_effort(svc: &Services, thread: &Thread) -> bool {
    svc.effort_store
        .stream_has_open_effort(thread.stream_id)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "filing: open-effort lookup failed");
            false
        })
}

/// Returns true when the worktree is mid git merge / rebase /
/// cherry-pick / revert. Filing enforcement exempts edits in these
/// states because conflict resolution can't dead-lock against the
/// filing rule. Mirrors `src/electron/filing-enforcement.ts`.
pub fn git_operation_in_progress(project_dir: &Path) -> bool {
    let gitdir = project_dir.join(".git");
    for marker in [
        "MERGE_HEAD",
        "REBASE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
    ] {
        if gitdir.join(marker).exists() {
            return true;
        }
    }
    // Worktrees: .git is a file pointing at the real gitdir.
    if let Ok(contents) = std::fs::read_to_string(&gitdir) {
        if let Some(real_dir) = contents.strip_prefix("gitdir: ") {
            let real = Path::new(real_dir.trim());
            for marker in [
                "MERGE_HEAD",
                "REBASE_HEAD",
                "CHERRY_PICK_HEAD",
                "REVERT_HEAD",
            ] {
                if real.join(marker).exists() {
                    return true;
                }
            }
        }
    }
    false
}

/// The stream's open efforts, named for the Stop audit: an oxplow task's
/// by its id and title (its `updated_at` + note count as the stamp that
/// re-arms the audit), another provider's item by its label.
async fn open_efforts_in_stream(svc: &Services, thread: &Thread) -> Vec<OpenEffort> {
    let efforts = svc
        .effort_store
        .list_open_for_stream(thread.stream_id)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "stop audit: open-effort lookup failed");
            Vec::new()
        });
    let mut out = Vec::with_capacity(efforts.len());
    for e in efforts {
        let label = oxplow_domain::refs::build::work_item_label(&e.work_item);
        let task = match e.task_id() {
            Some(id) => svc.task_store.get(id).await.ok().flatten(),
            None => None,
        };
        let (title, stamp) = match task {
            Some(t) => (
                t.title.clone(),
                format!("{}|{}", t.updated_at.unix_ms(), t.note_count),
            ),
            None => (label.clone(), String::new()),
        };
        out.push(OpenEffort {
            effort_id: e.id.to_string(),
            label,
            title,
            stamp,
        });
    }
    out
}

fn build_open_effort_audit_reason(efforts: &[OpenEffort]) -> String {
    let lines: Vec<String> = efforts
        .iter()
        .map(|e| {
            if e.title == e.label {
                format!("  • [{}] {}", e.effort_id, e.label)
            } else {
                format!("  • [{}] {} — {}", e.effort_id, e.label, e.title)
            }
        })
        .collect();
    format!(
        "AUDIT: this turn is closing with {} effort(s) still open in this stream:\n{}\n\n\
         Before stopping, walk each one:\n\
         - An oxplow task (tsk…) done? → `mcp__oxplow__run_command` `command.sequence` of \
           `work_item.transition` (`\"to\": \"done\"`) and `effort.report` (`summary`, \
           `touched_files`). Stale or no longer the right shape? → `work_item.transition` to \
           todo/blocked/canceled.\n\
         - Another provider's work item? → `run_command` `effort.close` with \
           `{{\"effort\": \"eff…\"}}` when it's done.\n\
         - Waiting on the user? → `mcp__oxplow__await_user`.\n\n\
         An open effort with finished work parked in it looks stuck to the user.",
        efforts.len(),
        lines.join("\n")
    )
}

/// Compose the human descriptor for a run from its already-fetched parts. Pure
/// so it's unit-testable; [`describe_run`] does the I/O and calls this (tsk266).
/// The `run:<id>` ref leads so `claim_runs`/`disclaim_runs` can still parse it.
fn format_run_descriptor(
    run_ref: &str,
    summary: Option<&str>,
    time_hm: Option<(u8, u8)>,
) -> String {
    let mut out = run_ref.to_string();
    if let Some(s) = summary.map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(&format!(" — {s}"));
    }
    if let Some((h, m)) = time_hm {
        out.push_str(&format!(" @ {h:02}:{m:02}"));
    }
    out
}

/// Render the kind-specific middle of a run descriptor from its `*-detail`
/// finding payload — tests show command + pass/fail, coverage shows the diff %,
/// analysis shows the analyzer + error/warning counts (tsk269). Pure for testing.
fn run_summary_from_detail(detail_kind: &str, payload: &serde_json::Value) -> Option<String> {
    let counts = |parts: &[(&str, &str)]| -> String {
        let joined: Vec<String> = parts
            .iter()
            .filter_map(|(key, label)| {
                payload
                    .get(*key)
                    .and_then(serde_json::Value::as_i64)
                    .map(|n| format!("{n} {label}"))
            })
            .collect();
        if joined.is_empty() {
            String::new()
        } else {
            format!(" ({})", joined.join(", "))
        }
    };
    let str_field = |key: &str| payload.get(key).and_then(|v| v.as_str());
    match detail_kind {
        "test-detail" => {
            let cmd = str_field("command").unwrap_or("test run").trim();
            Some(format!(
                "{cmd}{}",
                counts(&[("passed", "passed"), ("failed", "failed")])
            ))
        }
        "coverage-detail" => Some(
            match payload
                .get("summaryPct")
                .and_then(serde_json::Value::as_f64)
            {
                Some(pct) => format!("coverage ({pct:.0}% of changed lines)"),
                None => "coverage report".to_string(),
            },
        ),
        "analysis-detail" => {
            let who = str_field("analyzer")
                .or_else(|| str_field("command"))
                .unwrap_or("analysis")
                .trim();
            Some(format!(
                "{who}{}",
                counts(&[("errorCount", "errors"), ("warningCount", "warnings")])
            ))
        }
        _ => None,
    }
}

/// Turn a bare `run:<id>` ledger ref into a descriptor by joining the
/// `metric_run` row (timestamp) + its `*-detail` finding (test/coverage/analysis)
/// from the substrate. Dispatches on the finding kind so a coverage/analysis run
/// renders as such, not a malformed test run (tsk266/tsk269). Falls back to the
/// bare ref when the run can't be looked up — never blocks the review.
async fn describe_run(facts: &oxplow_db::SqliteFactStore, run_ref: &str) -> String {
    let Some(id) = run_ref
        .strip_prefix("run:")
        .and_then(|s| s.parse::<i64>().ok())
    else {
        return run_ref.to_string();
    };
    // The capture IS the run (T-E1, tsk48): the verbatim payload rides in its
    // `detail_json` envelope `{"kind": …, "payload": …}`.
    let capture = facts.get_capture(id).await.ok().flatten();
    let summary = capture.as_ref().and_then(|c| {
        let envelope = serde_json::from_str::<serde_json::Value>(c.detail_json.as_deref()?).ok()?;
        run_summary_from_detail(envelope["kind"].as_str()?, envelope.get("payload")?)
    });
    let time_hm = capture
        .as_ref()
        .map(|c| (c.captured_at.0.hour(), c.captured_at.0.minute()));
    format_run_descriptor(run_ref, summary.as_deref(), time_hm)
}

fn build_effort_file_review_reason(reviews: &[PendingEffortReview]) -> String {
    let mut out = String::from(
        "EFFORT REVIEW: one or more efforts you just closed have a discrepancy between \
         what you declared and what oxplow observed — in the files you touched and/or \
         the test/coverage/analysis runs that happened during your effort. For each:\n\n",
    );
    for r in reviews {
        // Canonical ids (`tsk42`, `eff313`): what `work_item.*` /
        // `effort.amend` parse, so the agent can paste them back (tsk341).
        if r.work_item.is_empty() {
            out.push_str(&format!("  • {} (effort {})\n", r.task_title, r.effort_id));
        } else {
            out.push_str(&format!(
                "  • [{}] {} (effort {})\n",
                oxplow_domain::refs::build::work_item_label(&r.work_item),
                r.task_title,
                r.effort_id
            ));
        }
        if !r.claimed_but_not_changed.is_empty() {
            out.push_str("      You claimed these files but the worktree didn't change:\n");
            for p in &r.claimed_but_not_changed {
                out.push_str(&format!("        - {p}\n"));
            }
        }
        if !r.changed_but_not_claimed.is_empty() {
            out.push_str(
                "      These files changed during your effort but you didn't list them:\n",
            );
            for p in &r.changed_but_not_claimed {
                out.push_str(&format!("        - {p}\n"));
            }
        }
        if let Some(total) = r.unclaimed_overflow {
            out.push_str(&format!(
                "      ({total} files changed during your effort that you didn't claim — \
                 too many to triage; skipping. Likely from another effort, formatter, \
                 or external activity.)\n"
            ));
        }
        if !r.unattributed_runs.is_empty() {
            out.push_str(
                "      These test/coverage/analysis runs happened during your effort but \
                 weren't attributed to you (a concurrent effort was open, so oxplow couldn't \
                 auto-assign them):\n",
            );
            for run in &r.unattributed_runs {
                out.push_str(&format!("        - {run}\n"));
            }
        }
    }
    out.push_str(
        "\nIf any are wrong, run `effort.amend { effort, add_files, remove_files, \
         claim_runs, disclaim_runs }` (`mcp__oxplow__run_command`) to correct — \
         `claim_runs` for runs that were yours, `disclaim_runs` for ones that weren't. If your original \
         declaration was right (you reverted an edit, or another actor/effort produced \
         those changes/runs), no amend is needed — silent agreement is fine and the \
         prompt won't repeat.",
    );
    out
}

fn build_filed_but_didnt_ship_reason() -> String {
    "FILED BUT DIDN'T SHIP: you filed a `ready` task this turn but didn't open one as `in_progress` and didn't make any code edits. \
     If you meant to start the work, mark one in_progress and re-issue the edit. \
     If you meant to queue it for later, reply with that intent and the next turn picks it up."
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::EffortStore as _;

    /// tsk468: the EFFORT REVIEW names an effort and its work item by the
    /// canonical ids `effort.amend` and `work_item.*` take, so an agent
    /// can paste them straight back — as the file review that fills them
    /// writes them (tsk935), not as a test spells them.
    #[test]
    fn the_effort_review_prints_canonical_ids() {
        let reviewed = crate::task_service::review_from_lists(
            &oxplow_domain::EffortId::new(313),
            &oxplow_domain::refs::build::work_item_ref(oxplow_domain::TaskId::new(42)),
            &["src/a.rs".into()],
            &[],
            &[],
            &[],
        )
        .expect("a claimed file that didn't change");
        let review = PendingEffortReview {
            effort_id: reviewed.effort_id,
            work_item: reviewed.work_item,
            task_title: "Fix the thing".into(),
            claimed_but_not_changed: reviewed.claimed_but_not_changed,
            changed_but_not_claimed: reviewed.changed_but_not_claimed,
            unclaimed_overflow: reviewed.unclaimed_overflow,
            unattributed_runs: vec![],
        };
        let text = build_effort_file_review_reason(&[review]);
        assert!(
            text.contains("  • [tsk42] Fix the thing (effort eff313)\n"),
            "{text}"
        );
    }

    /// P3.4 (tsk474): a turn's signals are the tool events anchored to that
    /// turn — a read-only turn, a writing turn and a Q&A turn each read
    /// alone, whatever ran in the turns around them.
    #[tokio::test]
    async fn turn_signals_come_from_the_turns_own_tool_events() {
        use crate::{HookEnvelope, ToolDecision};
        use oxplow_domain::HookKind;
        let f = crate::test_fixtures::services_with_effort().await;
        let hook = |kind: HookKind, body: serde_json::Value| HookEnvelope {
            kind,
            thread_id: Some(f.thread),
            stream_id: None,
            session_id: Some("s".into()),
            payload_json: body.to_string(),
            prompt: Some("go".into()),
            decision: (kind == HookKind::PreToolUse).then_some(ToolDecision {
                allowed: true,
                reason: None,
            }),
        };
        let mut seen = Vec::new();
        for tools in [
            vec![(HookKind::PostToolUse, "Read")],
            vec![
                (HookKind::PreToolUse, "Edit"),
                (HookKind::PostToolUse, "Edit"),
            ],
            vec![],
        ] {
            f.svc
                .hook_ingest
                .ingest(hook(HookKind::UserPromptSubmit, serde_json::json!({})))
                .await
                .unwrap();
            for (kind, tool) in tools {
                f.svc
                    .hook_ingest
                    .ingest(hook(
                        kind,
                        serde_json::json!({"tool_name": tool, "tool_input": {"file_path": "a.rs"}}),
                    ))
                    .await
                    .unwrap();
            }
            let stop = f
                .svc
                .hook_ingest
                .ingest(hook(HookKind::Stop, serde_json::json!({})))
                .await
                .unwrap();
            let s = TurnSignals::of_turn(&f.svc.db, stop.closed_turn.unwrap())
                .await
                .unwrap();
            seen.push((s.had_activity, s.had_writes));
        }
        assert_eq!(seen, vec![(true, false), (true, true), (false, false)]);
    }
    /// A write the policy refused still counts as the turn trying to change
    /// files.
    #[tokio::test]
    async fn a_denied_write_counts_as_a_write() {
        use crate::{HookEnvelope, ToolDecision};
        use oxplow_domain::HookKind;
        let f = crate::test_fixtures::services_with_effort().await;
        let hook = |kind: HookKind, body: serde_json::Value, decision| HookEnvelope {
            kind,
            thread_id: Some(f.thread),
            stream_id: None,
            session_id: Some("s".into()),
            payload_json: body.to_string(),
            prompt: Some("go".into()),
            decision,
        };
        f.svc
            .hook_ingest
            .ingest(hook(
                HookKind::UserPromptSubmit,
                serde_json::json!({}),
                None,
            ))
            .await
            .unwrap();
        f.svc
            .hook_ingest
            .ingest(hook(
                HookKind::PreToolUse,
                serde_json::json!({"tool_name": "Edit", "tool_input": {"file_path": "a.rs"}}),
                Some(ToolDecision {
                    allowed: false,
                    reason: Some("no effort".into()),
                }),
            ))
            .await
            .unwrap();
        let stop = f
            .svc
            .hook_ingest
            .ingest(hook(HookKind::Stop, serde_json::json!({}), None))
            .await
            .unwrap();
        let s = TurnSignals::of_turn(&f.svc.db, stop.closed_turn.unwrap())
            .await
            .unwrap();
        assert!(s.had_activity && s.had_writes);
    }

    /// The Stop gates read the turn from the log (tsk504): a turn that
    /// parked on the person, or left a subagent running, suppresses the
    /// Stop directives even with in_progress work open.
    #[tokio::test]
    async fn awaiting_user_and_a_running_subagent_come_from_the_turn() {
        use crate::{HookEnvelope, ToolDecision};
        use oxplow_domain::{AgentStatusState, HookKind};
        let f = crate::test_fixtures::services_with_effort().await;
        let hook = |kind: HookKind, body: serde_json::Value| HookEnvelope {
            kind,
            thread_id: Some(f.thread),
            stream_id: None,
            session_id: Some("s".into()),
            payload_json: body.to_string(),
            prompt: Some("go".into()),
            decision: (kind == HookKind::PreToolUse).then_some(ToolDecision {
                allowed: true,
                reason: None,
            }),
        };
        let ingest = |env: HookEnvelope| {
            let svc = f.svc.clone();
            async move { svc.hook_ingest.ingest(env).await.unwrap() }
        };
        let tool = |t: &str| serde_json::json!({"tool_name": t, "tool_input": {}});

        // A turn that asks the person something.
        ingest(hook(HookKind::UserPromptSubmit, serde_json::json!({}))).await;
        ingest(hook(HookKind::PreToolUse, tool("mcp__oxplow__await_user"))).await;
        f.svc
            .hook_ingest
            .set_status(
                &f.thread,
                AgentStatusState::AwaitingUser,
                Some("A or B?".into()),
            )
            .await
            .unwrap();
        let asked = ingest(hook(HookKind::Stop, serde_json::json!({}))).await;
        let asked = TurnSignals::of_turn(&f.svc.db, asked.closed_turn.unwrap())
            .await
            .unwrap();
        assert!(asked.awaiting_user && !asked.subagent_in_flight);
        assert!(
            f.svc
                .agent_policy
                .on_turn_end(&f.svc, &f.thread, Some(&asked))
                .await
                .is_none(),
            "a turn waiting on the person gets no directive"
        );

        // A turn whose subagent is still running, then one whose finished.
        let mut seen = Vec::new();
        for finished in [false, true] {
            ingest(hook(HookKind::UserPromptSubmit, serde_json::json!({}))).await;
            ingest(hook(HookKind::PreToolUse, tool("Task"))).await;
            if finished {
                ingest(hook(HookKind::PostToolUse, tool("Task"))).await;
            }
            let stop = ingest(hook(HookKind::Stop, serde_json::json!({}))).await;
            let s = TurnSignals::of_turn(&f.svc.db, stop.closed_turn.unwrap())
                .await
                .unwrap();
            seen.push((s.awaiting_user, s.subagent_in_flight));
        }
        assert_eq!(seen, vec![(false, true), (false, false)]);
    }

    use oxplow_domain::refs::build::work_item_ref;
    use oxplow_domain::stores::StreamStore;
    use oxplow_domain::{
        AgentKind, Stream, StreamId, StreamKind, Task, TaskActorKind, TaskId, TaskPriority,
        TaskStatus, ThreadStatus, Timestamp,
    };

    /// In-memory services over a real git repo (the session layer refuses
    /// non-git dirs).
    fn services() -> (Services, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        git2::Repository::init(dir.path()).unwrap();
        (Services::in_memory(dir.path()).unwrap(), dir)
    }

    fn test_stream() -> Stream {
        Stream {
            id: StreamId::placeholder(),
            kind: StreamKind::Worktree,
            title: "feat".into(),
            branch: "feat".into(),
            branch_ref: "refs/heads/feat".into(),
            branch_source: "main".into(),
            worktree_path: "/repo/wt".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        }
    }

    fn test_thread(stream_id: StreamId, status: ThreadStatus) -> Thread {
        Thread {
            id: ThreadId::placeholder(),
            stream_id,
            title: "thread".into(),
            status,
            sort_index: 0,
            pane_target: "working".into(),
            agent: AgentKind::Claude,
            acp_agent: None,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
            archived_at: None,
        }
    }

    fn in_progress_task(thread_id: ThreadId) -> Task {
        Task {
            id: TaskId::placeholder(),
            thread_id: Some(thread_id),
            parent_id: None,
            title: "work".into(),
            description: String::new(),
            status: TaskStatus::InProgress,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: None,
        }
    }

    /// The primary stream and its seeded active (writer) thread, which
    /// `Services::in_memory` creates via `ensure_primary` at boot.
    async fn primary_writer(svc: &Services) -> (Stream, Thread) {
        let stream = svc.stream_store.primary().await.unwrap().unwrap();
        let writer = svc
            .thread_store
            .list_for_stream(&stream.id)
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.status == ThreadStatus::Active)
            .expect("primary stream has a seeded active writer thread");
        (stream, writer)
    }

    /// A non-string `file_path` doesn't hide the path under another key
    /// (tsk371).
    #[test]
    fn claude_intent_takes_the_first_string_path() {
        let i = claude_intent(&serde_json::json!({
            "tool_name": "NotebookEdit",
            "tool_input": {"file_path": null, "notebook_path": "nb.ipynb"}
        }))
        .unwrap();
        assert_eq!(i.paths, vec!["nb.ipynb".to_string()]);
    }

    #[test]
    fn claude_intent_gates_exactly_the_guarded_tools() {
        // Only the four structured edits can be refused; everything else
        // skips the policy's lookups (tsk: pre_tool_check fast path).
        for t in ["Write", "Edit", "MultiEdit", "NotebookEdit"] {
            let i = claude_intent(
                &serde_json::json!({"tool_name": t, "tool_input": {"file_path": "src/a.rs"}}),
            )
            .unwrap();
            assert_eq!(
                (i.label.as_str(), i.kind, i.paths.clone()),
                (t, IntentKind::WorktreeWrite, vec!["src/a.rs".to_string()])
            );
        }
        for t in [
            "Read",
            "Grep",
            "Glob",
            "Bash",
            "Task",
            "WebFetch",
            "WebSearch",
            "TodoWrite",
            "mcp__oxplow__run_command",
            "",
        ] {
            assert!(
                claude_intent(&serde_json::json!({"tool_name": t})).is_none(),
                "{t} must short-circuit"
            );
        }
        // The gate admits exactly the union of the two guards' tool sets.
        use oxplow_runtime::filing::ALWAYS_WRITE_INTENT_TOOL_NAMES;
        use oxplow_runtime::write_guard::WORKTREE_MUTATING_TOOLS;
        for t in WORKTREE_MUTATING_TOOLS
            .iter()
            .chain(ALWAYS_WRITE_INTENT_TOOL_NAMES.iter())
        {
            assert!(
                claude_intent(&serde_json::json!({"tool_name": t})).is_some(),
                "gate must admit {t}"
            );
        }
        // notebook_path / path are read when file_path is absent.
        let nb = claude_intent(&serde_json::json!({"tool_name": "NotebookEdit", "tool_input": {"notebook_path": "n.ipynb"}})).unwrap();
        assert_eq!(nb.paths, vec!["n.ipynb".to_string()]);
    }

    #[test]
    fn format_run_descriptor_renders_summary_and_time() {
        // tsk266: the agent gets a recognizable line, not an opaque id.
        assert_eq!(
            format_run_descriptor(
                "run:47",
                Some("cargo test (419 passed, 0 failed)"),
                Some((10, 3))
            ),
            "run:47 — cargo test (419 passed, 0 failed) @ 10:03"
        );
        // Every piece is optional; the ref always leads so claim_runs can parse it.
        assert_eq!(format_run_descriptor("run:9", None, None), "run:9");
        assert_eq!(format_run_descriptor("run:9", Some("   "), None), "run:9");
    }

    #[test]
    fn run_summary_dispatches_per_detail_kind() {
        // tsk269: a coverage/analysis run renders as such, not a malformed test.
        let test = serde_json::json!({"command": "cargo test", "passed": 419, "failed": 0});
        assert_eq!(
            run_summary_from_detail("test-detail", &test).as_deref(),
            Some("cargo test (419 passed, 0 failed)")
        );
        let cov = serde_json::json!({"summaryPct": 83.4});
        assert_eq!(
            run_summary_from_detail("coverage-detail", &cov).as_deref(),
            Some("coverage (83% of changed lines)")
        );
        let analysis =
            serde_json::json!({"analyzer": "clippy", "errorCount": 0, "warningCount": 2});
        assert_eq!(
            run_summary_from_detail("analysis-detail", &analysis).as_deref(),
            Some("clippy (0 errors, 2 warnings)")
        );
        assert_eq!(run_summary_from_detail("other", &test), None);
    }

    #[test]
    fn git_op_in_progress_detects_merge_head() {
        use std::fs;
        let tmp = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".git")).unwrap();
        assert!(!git_operation_in_progress(tmp.path()));
        fs::write(tmp.path().join(".git/MERGE_HEAD"), b"deadbeef\n").unwrap();
        assert!(git_operation_in_progress(tmp.path()));
    }

    #[test]
    fn git_op_in_progress_detects_each_marker() {
        use std::fs;
        for marker in ["REBASE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"] {
            let tmp = tempfile::TempDir::new().unwrap();
            fs::create_dir_all(tmp.path().join(".git")).unwrap();
            assert!(!git_operation_in_progress(tmp.path()));
            fs::write(tmp.path().join(".git").join(marker), b"deadbeef\n").unwrap();
            assert!(
                git_operation_in_progress(tmp.path()),
                "expected {marker} to count"
            );
        }
    }

    #[test]
    fn git_op_in_progress_follows_worktree_gitdir_pointer() {
        // In a secondary worktree, `.git` is a *file* pointing at the
        // real gitdir. The function must follow that pointer so a
        // mid-merge worktree still trips the carve-out.
        use std::fs;
        let tmp = tempfile::TempDir::new().unwrap();
        let real_gitdir = tmp.path().join("real-gitdir");
        let worktree = tmp.path().join("worktree");
        fs::create_dir_all(&real_gitdir).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", real_gitdir.display()),
        )
        .unwrap();
        assert!(!git_operation_in_progress(&worktree));
        fs::write(real_gitdir.join("MERGE_HEAD"), b"x\n").unwrap();
        assert!(git_operation_in_progress(&worktree));
    }

    #[test]
    fn git_op_in_progress_no_dot_git_returns_false() {
        // Bare directory with no .git at all — function must not
        // panic and must report no-op.
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(!git_operation_in_progress(tmp.path()));
    }

    /// P2.7 (tsk431): the claim is an OPEN EFFORT in the stream, on any of
    /// its threads (tsk133 — cross-thread dispatch keeps working).
    #[tokio::test]
    async fn an_effort_on_a_sibling_thread_satisfies_the_writer() {
        let (svc, _dir) = services();
        let (stream, writer_b) = primary_writer(&svc).await;
        let sibling_a_id = svc
            .thread_store
            .upsert(&test_thread(stream.id, ThreadStatus::Queued))
            .await
            .unwrap();
        let task = svc
            .task_store
            .insert(&in_progress_task(sibling_a_id))
            .await
            .unwrap();
        svc.effort_store
            .start(&work_item_ref(task), &sibling_a_id, None)
            .await
            .unwrap();
        assert!(stream_has_open_effort(&svc, &writer_b).await);
    }

    /// Another provider's work item is a claim too (`effort.open`).
    #[tokio::test]
    async fn an_effort_on_a_foreign_work_item_satisfies_the_writer() {
        let (svc, _dir) = services();
        let (_stream, writer) = primary_writer(&svc).await;
        svc.effort_store
            .start("work_item:linear:ENG-12", &writer.id, None)
            .await
            .unwrap();
        assert!(stream_has_open_effort(&svc, &writer).await);
    }

    /// An `in_progress` row alone is not a claim — only its effort is.
    #[tokio::test]
    async fn an_in_progress_task_without_an_effort_does_not_satisfy() {
        let (svc, _dir) = services();
        let (_stream, writer) = primary_writer(&svc).await;
        svc.task_store
            .insert(&in_progress_task(writer.id))
            .await
            .unwrap();
        assert!(!stream_has_open_effort(&svc, &writer).await);
    }

    #[tokio::test]
    async fn an_effort_in_another_stream_does_not_satisfy() {
        let (svc, _dir) = services();
        let (_stream, writer) = primary_writer(&svc).await;
        let other_stream_id = svc.stream_store.upsert(&test_stream()).await.unwrap();
        let other_thread_id = svc
            .thread_store
            .upsert(&test_thread(other_stream_id, ThreadStatus::Active))
            .await
            .unwrap();
        svc.effort_store
            .start("work_item:linear:ENG-9", &other_thread_id, None)
            .await
            .unwrap();
        assert!(!stream_has_open_effort(&svc, &writer).await);
    }

    #[tokio::test]
    async fn no_claim_anywhere_does_not_satisfy() {
        let (svc, _dir) = services();
        let (_stream, writer) = primary_writer(&svc).await;
        assert!(
            !stream_has_open_effort(&svc, &writer).await,
            "no open effort anywhere → guard not satisfied"
        );
    }
}
