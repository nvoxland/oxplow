import { test, expect } from "bun:test";
import { execFileSync, spawnSync } from "node:child_process";
import { hasSession, killSession, watchSession } from "./tmux.js";

function tmuxAvailable(): boolean {
  try {
    execFileSync("tmux", ["-V"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function createTestSession(name: string) {
  execFileSync("tmux", ["new-session", "-d", "-s", name], { stdio: "ignore" });
}

function waitFor(condition: () => boolean, timeoutMs = 5000, intervalMs = 100): Promise<void> {
  return new Promise((resolve, reject) => {
    const deadline = Date.now() + timeoutMs;
    const check = () => {
      if (condition()) return resolve();
      if (Date.now() >= deadline) return reject(new Error("timed out waiting for condition"));
      setTimeout(check, intervalMs);
    };
    check();
  });
}

test("killSession removes a tmux session", () => {
  if (!tmuxAvailable()) return;
  const name = `newde-test-kill-${process.pid}`;
  createTestSession(name);
  expect(hasSession(name)).toBe(true);
  killSession(name);
  expect(hasSession(name)).toBe(false);
});

test("killSession is a no-op for a non-existent session", () => {
  if (!tmuxAvailable()) return;
  expect(() => killSession("newde-test-nonexistent-999999")).not.toThrow();
});

test("watchSession kills session when watched pid exits", async () => {
  if (!tmuxAvailable()) return;

  // Spawn a short-lived process so we have a real pid that will exit.
  const child = spawnSync("sh", ["-c", "sleep 0.1"], {});

  const name = `newde-test-watch-${process.pid}`;
  createTestSession(name);
  expect(hasSession(name)).toBe(true);

  // Watch using the now-dead pid (process already exited, kill -0 will fail immediately).
  watchSession(name, child.pid as number);

  // Sentinel polls every 2 s; since the pid is already gone it should fire quickly.
  await waitFor(() => !hasSession(name), 6000);
  expect(hasSession(name)).toBe(false);
}, 8000);
