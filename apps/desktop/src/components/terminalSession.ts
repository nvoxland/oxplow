/// How a terminal pane reads its session's messages and decides when to open
/// the session again (tsk1026): a session ends when its process exits, and is
/// gone after the daemon restarts — the pane says so, or opens it again,
/// instead of showing a dead screen that swallows keystrokes.

/** One message from the daemon about a pane's session. */
export type ExitOutcome =
  | { kind: "ended" }
  // The launch failed before any process existed (a program not installed,
  // the harness refusing): the reason, shown where "Session ended" is.
  | { kind: "launch_failed"; message: string }
  | { kind: "unable_to_resume"; reason: "exited_early" | "transcript_missing"; exitCode: number };

export type SessionMessage =
  | { kind: "data"; bytes: Uint8Array }
  | { kind: "exit"; exitCode: number | null; outcome: ExitOutcome };

function readOutcome(value: unknown): ExitOutcome {
  const o = value as { kind?: unknown; reason?: unknown; exitCode?: unknown } | null;
  if (o && o.kind === "unable_to_resume" && typeof o.exitCode === "number") {
    return {
      kind: "unable_to_resume",
      reason: o.reason === "transcript_missing" ? "transcript_missing" : "exited_early",
      exitCode: o.exitCode,
    };
  }
  return { kind: "ended" };
}

/** The session message in a terminal event's JSON, or `null` for anything
 *  else. */
export function readSessionMessage(message: string): SessionMessage | null {
  let msg: unknown;
  try {
    msg = JSON.parse(message);
  } catch {
    return null;
  }
  if (typeof msg !== "object" || msg === null) return null;
  const m = msg as { type?: unknown; bytes?: unknown; exitCode?: unknown };
  if (m.type === "data" && typeof m.bytes === "string") {
    const bin = atob(m.bytes);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    return { kind: "data", bytes };
  }
  if (m.type === "exit") {
    return {
      kind: "exit",
      exitCode: typeof m.exitCode === "number" ? m.exitCode : null,
      outcome: readOutcome((m as { outcome?: unknown }).outcome),
    };
  }
  return null;
}

/** After opening the pane's session again: `same` when the session the pane
 *  shows is still the one running (a reconnect that lost nothing), `new`
 *  when it was replaced and the screen must start over from its replay. */
export function reopened(current: string | null, opened: string): "same" | "new" {
  return current === opened ? "same" : "new";
}

/** The line written above a session that replaced one which ended. */
export const RESTARTED_NOTICE = "\x1b[2m— the previous session ended; this one was started again —\x1b[0m\r\n";

/** Whether a pane may spawn its process now. A pane that starts on
 *  activation (an agent session's tab) waits until its tab is shown: a
 *  restored tab nobody looked at starts nothing. Other panes (a shell)
 *  open at once. */
export function mayStartNow(startOnActivate: boolean, visible: boolean): boolean {
  return !startOnActivate || visible;
}

/** The pane state for an open that failed: it has no running process, and
 *  the banner says why. */
export function failedLaunch(error: unknown): { exitCode: null; outcome: ExitOutcome } {
  const message =
    error instanceof Error
      ? error.message
      : typeof (error as { message?: unknown } | null)?.message === "string"
        ? (error as { message: string }).message
        : String(error);
  return { exitCode: null, outcome: { kind: "launch_failed", message } };
}

/** What the ended banner says and offers. A resume that failed before a
 *  turn ran keeps the saved conversation: Retry resumes it again, Start
 *  fresh deliberately begins a new one. Any other end offers Start Again,
 *  which resumes the saved conversation as well. */
export function exitBanner(ended: { exitCode: number | null; outcome: ExitOutcome }): {
  text: string;
  actions: Array<"start-again" | "retry" | "start-fresh">;
} {
  const o = ended.outcome;
  if (o.kind === "launch_failed") {
    return { text: `Couldn't start the session: ${o.message}`, actions: ["retry"] };
  }
  if (o.kind === "unable_to_resume") {
    const why =
      o.reason === "transcript_missing"
        ? "the saved conversation's transcript wasn't found"
        : "it exited before the conversation started";
    return {
      text: `Couldn't resume the saved conversation (exit ${o.exitCode}): ${why}. It is kept — retry, or start a fresh one.`,
      actions: ["retry", "start-fresh"],
    };
  }
  return {
    text: `Session ended${ended.exitCode === null ? "" : ended.exitCode === 0 ? "" : ` (exit ${ended.exitCode})`}.`,
    actions: ["start-again"],
  };
}
