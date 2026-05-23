//! Shared helpers for running background git operations.
//!
//! A git op kicked off via `gitMergeInto` / `gitPush` / … returns a
//! `GitOpKickoff` whose `awaitDone` resolves to a `BackgroundTask`. Every
//! caller then has to normalize that task into a `GitOpResult` (the task
//! may have died before producing a result payload) and extract a
//! human-readable failure message. That normalization was copy-pasted
//! across ProjectPanel's push/pull dialog, BranchPicker's merge/rebase,
//! and the git dashboard's `runOp`. These pure helpers are the single
//! source of truth.
//!
//! Not a hook: the call sites surface errors differently (the dashboard
//! and ProjectPanel toast + record an op-error; BranchPicker shows an
//! inline message), so a stateful `useGitOps` would force a wrong shared
//! abstraction. The shared part is pure result-normalization.

import type { BackgroundTask, GitOpKickoff } from "./api.js";
import type { GitOpResult } from "./tauri-bridge/index.js";

/// Normalize a finished (or failed) background task into a `GitOpResult`.
/// When the task ended without a `result` payload (e.g. it errored or was
/// killed) we synthesize one: `success` follows the task status and the
/// task's `error` becomes the stderr so callers still get a message.
export function normalizeGitOpResult(task: BackgroundTask | null): GitOpResult {
  return (
    (task?.result as GitOpResult | undefined) ?? {
      success: task?.status === "done",
      stdout: "",
      stderr: task?.error ?? "",
      status: null,
    }
  );
}

/// Await a kicked-off git op and normalize its result. Replaces the
/// `const { awaitDone } = await op(); const task = await awaitDone; …`
/// dance at every call site.
export async function awaitGitOp(kickoff: GitOpKickoff): Promise<GitOpResult> {
  const task = await kickoff.awaitDone;
  return normalizeGitOpResult(task);
}

/// Best-effort failure message from a result: stderr, else stdout, else
/// the caller's fallback (e.g. "merge failed"). Trimmed.
export function gitOpErrorMessage(result: GitOpResult, fallback: string): string {
  return (result.stderr || result.stdout || fallback).trim();
}
