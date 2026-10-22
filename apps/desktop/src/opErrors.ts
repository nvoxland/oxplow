/**
 * Operations that failed in front of the person (a merge, a push, a save,
 * a query): recorded on the daemon as `oxplow.ui.report_error`, read back
 * as `v_op_error`. The model is the one record — the Alerts page, the bell
 * and the toasts read it live, and an agent reads the same rows — so the
 * list survives a reload. The person's marks are commands:
 * `oxplow.op_error.mark_seen` (they opened its output) and
 * `oxplow.op_error.dismiss` / `dismiss_all`. See `.context/usability.md`
 * "Async-op failures don't `alert`".
 */

import { useCallback, useEffect, useState } from "react";

import { querySql, readEventContent, runCommand } from "./api.js";
import { logUi, type UiLogLevel } from "./logger.js";
import { NO_READS, useRerunOnChange } from "./lens/lensRerun.js";
import type { Reads, SqlCell, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

export interface OpErrorInput {
  /** Short user-facing label, e.g. "Merge bugfixes into current". */
  label: string;
  /** Optional shell-style command preview, e.g. "git merge bugfixes". */
  command?: string;
  /** Captured stderr (preferred) — typically the most useful field. */
  stderr?: string;
  /** Captured stdout, if anything was emitted before failure. */
  stdout?: string;
  /** Numeric exit code, if known. */
  exitCode?: number | null;
  /** Free-form long message when no stderr is available (thrown Error etc.). */
  message?: string;
  /** Thread the operation was started from. When omitted, the thread on
   *  screen (`setOpErrorContext`); null is "no thread". */
  threadId?: string | null;
  /** Wall-clock duration in ms, if measured. */
  durationMs?: number;
  /** Signal name if the child was killed by signal (SIGKILL, SIGTERM, …). */
  signal?: string | null;
}

/** A failed operation as `v_op_error` holds it. */
export interface OpError {
  /** The `ui.op_failed` event (`v_op_error.event_id`). */
  id: string;
  /** When it failed, epoch ms. */
  at: number;
  label: string;
  command: string | null;
  message: string | null;
  exitCode: number | null;
  signal: string | null;
  durationMs: number | null;
  /** Bytes of captured output; null when nothing was captured. */
  outputSize: number | null;
  /** The person opened its output. */
  seen: boolean;
}

/** Where the app is: what an error from no named thread is attributed to. */
let context: { thread: string | null; stream: string | null } = { thread: null, stream: null };

/** The thread and stream on screen (App sets it as they change). */
export function setOpErrorContext(next: { thread: string | null; stream: string | null }): void {
  context = next;
}

/** The `oxplow.ui.report_error` input for `entry`, from where the app is. */
export function reportInput(entry: OpErrorInput, at: { thread: string | null; stream: string | null }): Record<string, unknown> {
  const input: Record<string, unknown> = { label: entry.label };
  if (entry.command) input.command = entry.command;
  if (entry.message) input.message = entry.message;
  if (entry.stderr) input.stderr = entry.stderr;
  if (entry.stdout) input.stdout = entry.stdout;
  if (entry.exitCode != null) input.exit_code = entry.exitCode;
  const thread = entry.threadId !== undefined ? entry.threadId : at.thread;
  // A thread names its own stream; from no thread, the one on screen does,
  // so its agent can read the output.
  if (thread !== null) input.thread = thread;
  else if (at.stream !== null) input.stream = at.stream;
  if (entry.signal) input.signal = entry.signal;
  if (entry.durationMs != null) input.duration_ms = Math.round(entry.durationMs);
  return input;
}

/**
 * Report `entry` with `run`, without waiting. A report that fails is
 * logged, never recorded as another op error: that would report itself
 * again, without end.
 */
export function reportOpErrorWith(
  run: (name: string, input: unknown) => Promise<unknown>,
  log: (level: UiLogLevel, message: string, context?: Record<string, unknown>) => void,
  entry: OpErrorInput,
  at: { thread: string | null; stream: string | null },
): void {
  run("oxplow.ui.report_error", reportInput(entry, at)).catch((error: unknown) => {
    log("warn", "couldn't report an op error to the daemon", {
      label: entry.label,
      error: error instanceof Error ? error.message : String(error),
    });
  });
}

/** Record an operation that failed in front of the person. */
export function recordOpError(entry: OpErrorInput): void {
  reportOpErrorWith(runCommand, logUi, entry, context);
}

/** `v_op_error` rows as failed operations. */
export function opErrorsFromResult(result: SqlQueryResult): OpError[] {
  const col = (name: string) => result.columns.indexOf(name);
  const at = (row: SqlCell[], name: string): SqlCell => row[col(name)] ?? null;
  const text = (v: SqlCell) => (v === null ? null : String(v));
  const num = (v: SqlCell) => (v === null ? null : Number(v));
  return result.rows.map((row) => ({
    id: String(at(row, "event_id")),
    at: Date.parse(String(at(row, "at"))),
    label: String(at(row, "label") ?? ""),
    command: text(at(row, "command")),
    message: text(at(row, "message")),
    exitCode: num(at(row, "exit_code")),
    signal: text(at(row, "signal")),
    durationMs: num(at(row, "duration_ms")),
    outputSize: num(at(row, "output_size")),
    seen: Number(at(row, "seen")) === 1,
  }));
}

/** The failed operations the person hasn't dismissed, newest first. */
export async function readOpErrors(): Promise<{ errors: OpError[]; reads: Reads }> {
  const res = await querySql(
    `SELECT event_id, at, label, command, message, exit_code, signal, duration_ms, output_size, seen
       FROM v_op_error
      WHERE dismissed = 0
      ORDER BY at DESC
      LIMIT 50`,
    [],
    1_000,
  );
  return { errors: opErrorsFromResult(res), reads: res.reads };
}

/** The failed operations the person hasn't dismissed, live. */
export function useOpErrors(): OpError[] {
  const [errors, setErrors] = useState<OpError[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    void readOpErrors()
      .then((r) => {
        setErrors(r.errors);
        setReads(r.reads);
      })
      // Logged, not recorded: a failed read of the errors would record
      // itself as one.
      .catch((e: unknown) => logUi("warn", "couldn't read the failed operations", { error: e instanceof Error ? e.message : String(e) }));
  }, []);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
  return errors;
}

/** A failed operation's captured output, or null when it has none (or
 *  retention removed it). */
export async function readOpErrorOutput(id: string): Promise<{ stderr: string | null; stdout: string | null } | null> {
  const body = await readEventContent(id, "output");
  if (!body) return null;
  try {
    const parsed = JSON.parse(body.text) as { stderr?: unknown; stdout?: unknown };
    return {
      stderr: typeof parsed.stderr === "string" ? parsed.stderr : null,
      stdout: typeof parsed.stdout === "string" ? parsed.stdout : null,
    };
  } catch {
    return { stderr: body.text, stdout: null };
  }
}

export async function markOpErrorSeen(id: string): Promise<void> {
  await runCommand("oxplow.op_error.mark_seen", { event: id });
}

export async function dismissOpError(id: string): Promise<void> {
  await runCommand("oxplow.op_error.dismiss", { event: id });
}

export async function dismissAllOpErrors(): Promise<void> {
  await runCommand("oxplow.op_error.dismiss_all", {});
}
