import { ensureSession, ensureWindow, killWindow, listWindows } from "./tmux.js";
import type { Stream } from "./stream-store.js";

export function ensureStreamSession(stream: Stream, projectDir: string) {
  const session = stream.panes.working.split(":")[0];
  ensureSession(session, projectDir);
}

/**
 * Create (or resize-to-match) a tmux window for a pane, sized to the client's
 * reported dimensions. The window runs `claude` in the project dir. The
 * session's placeholder window is killed once the first real window exists.
 */
export function ensureStreamPane(
  target: string,
  projectDir: string,
  cols: number,
  rows: number,
) {
  const session = target.split(":")[0];
  ensureSession(session, projectDir);
  ensureWindow(target, projectDir, "claude", cols, rows);

  const placeholder = `${session}:__placeholder__`;
  if (listWindows(session).includes("__placeholder__") && listWindows(session).length > 1) {
    killWindow(placeholder);
  }
}
