import { test, expect } from "bun:test";
import { execFileSync } from "node:child_process";
import { capturePaneHistory, hasSession, killSession } from "./tmux.js";

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
  const name = `oxplow-test-kill-${process.pid}`;
  createTestSession(name);
  expect(hasSession(name)).toBe(true);
  killSession(name);
  expect(hasSession(name)).toBe(false);
});

test("killSession is a no-op for a non-existent session", () => {
  if (!tmuxAvailable()) return;
  expect(() => killSession("oxplow-test-nonexistent-999999")).not.toThrow();
});

test("capturePaneHistory returns recent pane output", async () => {
  if (!tmuxAvailable()) return;

  const name = `oxplow-test-capture-${process.pid}`;
  try {
    execFileSync("tmux", ["new-session", "-d", "-s", name, "printf 'alpha\\nbeta\\n' && sleep 1"], { stdio: "ignore" });
    await waitFor(() => capturePaneHistory(`${name}:0`, 50).includes("alpha"), 3000, 100);
    const history = capturePaneHistory(`${name}:0`, 50);
    expect(history).toContain("alpha");
    expect(history).toContain("beta");
  } finally {
    killSession(name);
  }
}, 5000);
