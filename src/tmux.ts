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
) {
  if (hasWindow(target)) {
    resizeWindow(target, cols, rows);
    return;
  }
  const [session, window] = target.split(":");
  tmux([
    "new-window",
    "-d",
    "-t", session,
    "-n", window,
    "-c", cwd,
    command,
  ]);
  resizeWindow(target, cols, rows);
}

export function resizeWindow(target: string, cols: number, rows: number) {
  if (cols < 2 || rows < 2) return;
  try {
    tmux(["resize-window", "-t", target, "-x", String(cols), "-y", String(rows)]);
  } catch {}
}

export function killWindow(target: string) {
  try {
    tmux(["kill-window", "-t", target]);
  } catch {}
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
