/// How a terminal pane reads its session's messages and decides when to open
/// the session again (tsk1026): a session ends when its process exits, and is
/// gone after the daemon restarts — the pane says so, or opens it again,
/// instead of showing a dead screen that swallows keystrokes.

/** One message from the daemon about a pane's session. */
export type SessionMessage =
  | { kind: "data"; bytes: Uint8Array }
  | { kind: "exit"; exitCode: number | null };

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
    return { kind: "exit", exitCode: typeof m.exitCode === "number" ? m.exitCode : null };
  }
  return null;
}

/** Whether a failed send means the session is gone (the daemon restarted,
 *  or the session ended), so the pane should open it again. */
export function isSessionGone(error: unknown): boolean {
  return /terminal session not found/i.test(error instanceof Error ? error.message : String(error));
}

/** After opening the pane's session again: `same` when the session the pane
 *  shows is still the one running (a reconnect that lost nothing), `new`
 *  when it was replaced and the screen must start over from its replay. */
export function reopened(current: string | null, opened: string): "same" | "new" {
  return current === opened ? "same" : "new";
}

/** The line written above a session that replaced one which ended. */
export const RESTARTED_NOTICE = "\x1b[2m— the previous session ended; this one was started again —\x1b[0m\r\n";
