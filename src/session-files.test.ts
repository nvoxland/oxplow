import { test, expect } from "bun:test";
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { createSessionFiles, destroySessionFiles } from "./session-files.js";

test("createSessionFiles writes hooks.json and executable forwarder", () => {
  const s = createSessionFiles({ daemonPort: 17999, streamId: "s-1", pane: "working" });
  try {
    expect(existsSync(s.dir)).toBe(true);
    expect(existsSync(s.settingsPath)).toBe(true);
    expect(existsSync(s.forwarderPath)).toBe(true);

    const mode = statSync(s.forwarderPath).mode;
    expect(mode & 0o100).toBeTruthy();
    const forwarderBody = readFileSync(s.forwarderPath, "utf8");
    expect(forwarderBody).toContain("127.0.0.1:17999");
    expect(forwarderBody).toContain("/api/hook/");
    expect(forwarderBody).toContain("stream=s-1");
    expect(forwarderBody).toContain("pane=working");

    const settings = JSON.parse(readFileSync(s.settingsPath, "utf8"));
    expect(settings.hooks).toBeDefined();
    for (const event of [
      "PreToolUse",
      "PostToolUse",
      "UserPromptSubmit",
      "SessionStart",
      "SessionEnd",
      "Stop",
      "Notification",
    ]) {
      const entries = settings.hooks[event];
      expect(Array.isArray(entries)).toBe(true);
      const cmd = entries[0].hooks[0];
      expect(cmd.type).toBe("command");
      expect(cmd.command).toContain("hook-forward.sh");
      expect(cmd.command).toContain(event);
    }
  } finally {
    destroySessionFiles(s);
  }
});

test("destroySessionFiles removes the directory", () => {
  const s = createSessionFiles({ daemonPort: 17999, streamId: "s-1", pane: "working" });
  destroySessionFiles(s);
  expect(existsSync(s.dir)).toBe(false);
});

test("destroySessionFiles is idempotent", () => {
  const s = createSessionFiles({ daemonPort: 17999, streamId: "s-1", pane: "working" });
  destroySessionFiles(s);
  expect(() => destroySessionFiles(s)).not.toThrow();
});

test("hook forwarder exits successfully when the daemon endpoint is unavailable", () => {
  const s = createSessionFiles({ daemonPort: 1, streamId: "s-1", pane: "working" });
  try {
    const result = spawnSync(s.forwarderPath, ["SessionStart"], {
      input: JSON.stringify({ session_id: "s1" }),
      encoding: "utf8",
    });
    expect(result.status).toBe(0);
  } finally {
    destroySessionFiles(s);
  }
});
