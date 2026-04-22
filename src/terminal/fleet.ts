import { ensureSession, ensureWindow, killWindow, listWindows } from "./tmux.js";
import type { Logger } from "../core/logger.js";
import type { PaneKind, Stream } from "../persistence/stream-store.js";
import { createHash } from "node:crypto";

export function ensureStreamSession(stream: Stream) {
  const session = stream.panes.working.split(":")[0];
  ensureSession(session, stream.worktree_path);
}

export function ensureAgentPane(
  target: string,
  cwd: string,
  cols: number,
  rows: number,
  agentCommand: string,
  opts: { signatureSource?: string; logger?: Logger } = {},
): boolean {
  const session = target.split(":")[0];
  const logger = opts.logger;
  logger?.debug("ensuring agent pane", { session, target, cwd, cols, rows });
  ensureSession(session, cwd);
  // Signature detects "launcher config actually changed" — stuff that would
  // warrant killing a live agent and respawning. Callers can pass a stripped
  // form (e.g. without --resume <id>) so reconnecting to a thread whose live
  // agent published a new resume id doesn't look like a config change.
  const signature = launcherSignature(opts.signatureSource ?? agentCommand);
  const created = ensureWindow(target, cwd, agentCommand, cols, rows, signature);

  const placeholder = `${session}:__placeholder__`;
  if (listWindows(session).includes("__placeholder__") && listWindows(session).length > 1) {
    killWindow(placeholder);
    logger?.debug("removed placeholder window", { session });
  }
  return created;
}

function launcherSignature(agentCommand: string): string {
  return createHash("sha256").update(agentCommand).digest("hex");
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
  return ensureAgentPane(target, stream.worktree_path, cols, rows, claudeCommand, { logger });
}
