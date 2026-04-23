import { execFileSync } from "node:child_process";

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
  launcherSignature?: string,
): boolean {
  if (hasWindow(target)) {
    if (launcherSignature && readWindowSignature(target) !== launcherSignature) {
      killWindow(target);
    } else {
      resizeWindow(target, cols, rows);
      return false;
    }
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
  if (launcherSignature) {
    writeWindowSignature(target, launcherSignature);
  }
  // Explicit resize ensures the window matches even if default-size was ignored.
  resizeWindow(target, cols, rows);
  return true;
}

function readWindowSignature(target: string): string | null {
  try {
    const out = execFileSync(
      "tmux",
      ["show-options", "-w", "-v", "-t", target, "@oxplow_launcher_signature"],
      { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] },
    );
    const value = out.trim();
    return value || null;
  } catch {
    return null;
  }
}

function writeWindowSignature(target: string, launcherSignature: string): void {
  try {
    tmux(["set-option", "-w", "-t", target, "@oxplow_launcher_signature", launcherSignature]);
  } catch {}
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
  } catch {
    // tmux exits non-zero when the session is already gone — that's exactly
    // the desired end state, so the only "error" here is the no-op case.
  }
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
