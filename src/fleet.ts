import { ensureSession, ensureWindow, killWindow, listWindows } from "./tmux.js";
import type { Logger } from "./logger.js";
import type { PaneKind, Stream } from "./stream-store.js";

export function ensureStreamSession(stream: Stream) {
  const session = stream.panes.working.split(":")[0];
  ensureSession(session, stream.worktree_path);
}

/**
 * Create (or resize-to-match) a tmux window for a pane, sized to the client's
 * reported dimensions. The window runs `claude` in the project dir. The
 * session's placeholder window is killed once the first real window exists.
 */
export function ensureStreamPane(
  stream: Stream,
  pane: PaneKind,
  cols: number,
  rows: number,
  claudeCommand: string,
  logger?: Logger,
): boolean {
  const target = stream.panes[pane];
  const session = target.split(":")[0];
  logger?.debug("ensuring stream pane", { session, target, cwd: stream.worktree_path, cols, rows });
  ensureSession(session, stream.worktree_path);
  const created = ensureWindow(target, stream.worktree_path, claudeCommand, cols, rows);

  const placeholder = `${session}:__placeholder__`;
  if (listWindows(session).includes("__placeholder__") && listWindows(session).length > 1) {
    killWindow(placeholder);
    logger?.debug("removed placeholder window", { session });
  }
  return created;
}
