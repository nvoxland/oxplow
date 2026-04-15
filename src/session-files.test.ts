import { test, expect } from "bun:test";
import { existsSync, readFileSync, statSync } from "node:fs";
import { createSessionFiles, destroySessionFiles } from "./session-files.js";

test("createSessionFiles writes hooks.json and executable forwarder", () => {
  const s = createSessionFiles({ daemonPort: 17999 });
  try {
    expect(existsSync(s.dir)).toBe(true);
    expect(existsSync(s.settingsPath)).toBe(true);
    expect(existsSync(s.forwarderPath)).toBe(true);

    const mode = statSync(s.forwarderPath).mode;
    expect(mode & 0o100).toBeTruthy();
    const forwarderBody = readFileSync(s.forwarderPath, "utf8");
    expect(forwarderBody).toContain("127.0.0.1:17999");
    expect(forwarderBody).toContain("/api/hook/");

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
  const s = createSessionFiles({ daemonPort: 17999 });
  destroySessionFiles(s);
  expect(existsSync(s.dir)).toBe(false);
});

test("destroySessionFiles is idempotent", () => {
  const s = createSessionFiles({ daemonPort: 17999 });
  destroySessionFiles(s);
  expect(() => destroySessionFiles(s)).not.toThrow();
});
