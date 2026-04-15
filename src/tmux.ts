import { execFileSync, spawn } from "node:child_process";

function tmux(args: string[]): string {
  return execFileSync("tmux", args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

export function hasSession(session: string): boolean {
  try {
    execFileSync("tmux", ["has-session", "-t", session], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

export function ensureSession(session: string, cwd: string) {
  if (hasSession(session)) return;
  // Placeholder window holds the session alive until a real pane is created;
  // it's killed as soon as any real window exists.
  tmux(["new-session", "-d", "-s", session, "-c", cwd, "-n", "__placeholder__"]);
  // Keep each window at its explicit size instead of tracking the latest
  // client. We drive resizes explicitly via `resize-window` whenever a client
  // reports new dimensions, so claude's output is never re-wrapped out from
  // under it by a transient client size.
  try {
    tmux(["set-option", "-t", session, "window-size", "manual"]);
  } catch {}
}

export function hasWindow(target: string): boolean {
  const [session, window] = target.split(":");
  try {
    const out = execFileSync("tmux", ["list-windows", "-t", session, "-F", "#{window_name}"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    });
    return out.split("\n").includes(window);
  } catch {
    return false;
  }
}

export function ensureWindow(
  target: string,
  cwd: string,
  command: string,
  cols: number,
  rows: number,
): boolean {
  if (hasWindow(target)) {
    resizeWindow(target, cols, rows);
    return false;
  }
  const [session, window] = target.split(":");
  // Set default-size so the new window is created at the correct dimensions
  // before the command starts rendering. tmux new-window doesn't accept -x/-y
  // (unlike new-session), but new windows inherit the session's default-size.
  try {
    tmux(["set-option", "-t", session, "default-size", `${cols}x${rows}`]);
  } catch {}
  tmux([
    "new-window",
    "-d",
    "-t", session,
    "-n", window,
    "-c", cwd,
    command,
  ]);
  // Explicit resize ensures the window matches even if default-size was ignored.
  resizeWindow(target, cols, rows);
  return true;
}

export function resizeWindow(target: string, cols: number, rows: number) {
  if (cols < 2 || rows < 2) return;
  try {
    tmux(["resize-window", "-t", target, "-x", String(cols), "-y", String(rows)]);
  } catch {}
}

export function capturePaneHistory(target: string, lineCount = 5000): string {
  try {
    return execFileSync(
      "tmux",
      ["capture-pane", "-p", "-e", "-J", "-S", `-${lineCount}`, "-t", target],
      { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] },
    );
  } catch {
    return "";
  }
}

export function copyModePage(target: string, direction: "up" | "down") {
  try {
    tmux(["copy-mode", "-e", "-t", target]);
    if (direction === "up") {
      tmux(["send-keys", "-t", target, "-X", "page-up"]);
      return;
    }
    tmux(["send-keys", "-t", target, "-X", "page-down"]);
  } catch {}
}

export function copyModeScroll(target: string, lines: number) {
  if (lines === 0) return;
  try {
    tmux(["copy-mode", "-e", "-t", target]);
    tmux([
      "send-keys",
      "-N",
      String(Math.abs(lines)),
      "-t",
      target,
      "-X",
      lines < 0 ? "scroll-up" : "scroll-down",
    ]);
  } catch {}
}

export function exitCopyMode(target: string) {
  try {
    tmux(["send-keys", "-t", target, "-X", "cancel"]);
  } catch {}
}

export function killWindow(target: string) {
  try {
    tmux(["kill-window", "-t", target]);
  } catch {}
}

export function killSession(session: string) {
  try {
    tmux(["kill-session", "-t", session]);
  } catch {}
}

/**
 * Spawn a detached sentinel process that watches `daemonPid` and kills the
 * tmux session when that pid is no longer alive. Handles daemon crashes and
 * SIGKILL in addition to graceful shutdown.
 *
 * The sentinel is fully detached (unref'd) so it does not keep the daemon's
 * event loop alive.
 */
export function watchSession(session: string, daemonPid: number) {
  // Shell one-liner: poll every 2 s; when the pid is gone, kill the session.
  const script = `while kill -0 ${daemonPid} 2>/dev/null; do sleep 2; done; tmux kill-session -t ${session} 2>/dev/null`;
  const child = spawn("sh", ["-c", script], {
    detached: true,
    stdio: "ignore",
  });
  child.unref();
}

export function listWindows(session: string): string[] {
  try {
    const out = execFileSync(
      "tmux",
      ["list-windows", "-t", session, "-F", "#{window_name}"],
      { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] },
    );
    return out.split("\n").filter((w) => w.length > 0);
  } catch {
    return [];
  }
}
