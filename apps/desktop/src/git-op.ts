//! Shared helpers for running background VCS operations.
//!
//! A VCS command kicked off via `vcsMerge` / `vcsPush` / … returns a
//! `GitOpKickoff` whose `awaitDone` resolves to a `BackgroundTask` whose
//! `result` is the command's `OpOutcome` (P5.B6). Every caller normalizes
//! that task (it may have died before producing a result) and extracts a
//! human-readable failure message; these pure helpers are the single
//! source of truth.
//!
//! Not a hook: the call sites surface errors differently (the dashboard
//! and ProjectPanel toast + record an op-error; BranchPicker shows an
//! inline message), so a stateful `useGitOps` would force a wrong shared
//! abstraction. The shared part is pure result-normalization.

import type { BackgroundTask, GitOpKickoff, OpOutcome } from "./api.js";
import type { OpErrorInput } from "./components/opErrorsStore.js";

/// Normalize a finished (or failed) background task into an `OpOutcome`.
/// When the task ended without a `result` payload (the command was
/// refused or threw) we synthesize one: `success` follows the task status
/// and the task's `error` becomes the log so callers still get a message.
export function normalizeGitOpResult(task: BackgroundTask | null): OpOutcome {
  return (
    (task?.result as OpOutcome | undefined) ?? {
      success: task?.status === "done",
      log: task?.error ?? "",
      conflicts: [],
      auto_resolved: 0,
    }
  );
}

/// Await a kicked-off VCS op and normalize its result.
export async function awaitGitOp(kickoff: GitOpKickoff): Promise<OpOutcome> {
  const task = await kickoff.awaitDone;
  return normalizeGitOpResult(task);
}

/// Run a quick VCS command and settle it into an `OpOutcome`: a refusal
/// or failure (the command threw) becomes an unsuccessful outcome
/// carrying the reason, so a result view shows it like any other.
export async function settleGitOp(op: () => Promise<OpOutcome>): Promise<OpOutcome> {
  try {
    return await op();
  } catch (e) {
    return {
      success: false,
      log: e instanceof Error ? e.message : String(e),
      conflicts: [],
      auto_resolved: 0,
    };
  }
}

/// Failure message from a result: the conflicted paths when there are
/// any, else the provider's log, else the caller's fallback (e.g. "merge
/// failed"). Trimmed.
export function gitOpErrorMessage(result: OpOutcome, fallback: string): string {
  if (result.conflicts.length > 0) {
    return `Conflicts in ${result.conflicts.join(", ")}`;
  }
  return (result.log || fallback).trim();
}

/// Human-readable toast summary for a finished op. On success it appends
/// the smart-merge auto-resolved count when the pass cleaned up any
/// conflicts ("… — 2 conflicts auto-resolved"); on failure it reports the
/// op as failed (the caller surfaces the details separately). `label` is
/// the verb phrase, e.g. "Cherry-pick a1b2c3d".
export function gitOpOutcomeMessage(label: string, result: OpOutcome): string {
  if (!result.success) return `${label} failed`;
  const resolved = result.auto_resolved;
  if (resolved > 0) {
    return `${label} succeeded — ${resolved} conflict${resolved === 1 ? "" : "s"} auto-resolved`;
  }
  return `${label} succeeded`;
}

/// The op-error record for a failed op: the conflicts and the provider's
/// log as the detail, `blankFailure` when neither says anything.
export function opErrorOf(label: string, command: string, result: OpOutcome): OpErrorInput {
  const detail = [
    result.conflicts.length > 0 ? `Conflicts in ${result.conflicts.join(", ")}` : "",
    result.log.trim(),
  ]
    .filter(Boolean)
    .join("\n");
  return { label, command, stderr: detail, blankFailure: detail === "" };
}
