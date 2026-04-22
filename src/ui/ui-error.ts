import { logUi } from "./logger.js";

/**
 * Lightweight pub/sub for surfacing user-facing IPC failures. Components
 * call `runWithError("Save thread", promise)` instead of `void
 * promise.catch(() => {})`; the App subscribes once and routes the message
 * to its visible error banner.
 *
 * Why a module-level emitter and not React context:
 * - Most callers are deep child components calling IPC fire-and-forget
 *   inside a click handler. Threading a `report` callback through
 *   props/context would touch every component on the way down.
 * - The error state still lives in App; this just decouples the
 *   "something failed" signal from the wiring.
 */

export interface UiErrorReport {
  label: string;
  message: string;
  cause?: unknown;
}

type Listener = (report: UiErrorReport) => void;

const listeners = new Set<Listener>();

export function subscribeUiError(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function reportUiError(label: string, cause: unknown): void {
  const message = cause instanceof Error ? cause.message : String(cause);
  logUi("warn", "ui error reported", { label, message });
  const report: UiErrorReport = { label, message, cause };
  for (const listener of [...listeners]) {
    try { listener(report); } catch {}
  }
}

/**
 * Run a fire-and-forget promise, routing any rejection to the UI error
 * banner. Use this anywhere you would have written
 * `void op.catch(() => {})`.
 *
 *   runWithError("Create commit point", createCommitPoint(...));
 */
export function runWithError<T>(label: string, promise: Promise<T>): void {
  // Catch the common bug of passing a thunk instead of a started promise:
  //   runWithError("X", () => doWork())   // type-loose, silently no-ops
  // The runtime check fires immediately so the bug surfaces in dev.
  if (typeof promise === "function" || promise == null || typeof (promise as { then?: unknown }).then !== "function") {
    const err = new Error(`runWithError("${label}") expected a Promise, got ${typeof promise}. Did you pass a thunk like () => fn() instead of fn()?`);
    reportUiError(label, err);
    return;
  }
  promise.catch((cause) => reportUiError(label, cause));
}
