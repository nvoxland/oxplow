import { test, expect } from "bun:test";
import { existsSync, mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createElectronPlugin } from "./claude-plugin.js";

test("createElectronPlugin writes a valid Claude Code plugin under .newde/runtime/claude-plugin", () => {
  const projectDir = mkdtempSync(join(tmpdir(), "newde-project-"));
  const plugin = createElectronPlugin({
    projectDir,
    hookUrl: "http://127.0.0.1:12345/hook",
  });

  expect(plugin.pluginDir).toBe(join(projectDir, ".newde", "runtime", "claude-plugin"));
  expect(plugin.manifestPath).toBe(join(plugin.pluginDir, ".claude-plugin", "plugin.json"));
  expect(plugin.hooksPath).toBe(join(plugin.pluginDir, "hooks", "hooks.json"));
  expect(existsSync(plugin.manifestPath)).toBe(true);
  expect(existsSync(plugin.hooksPath)).toBe(true);

  const manifest = JSON.parse(readFileSync(plugin.manifestPath, "utf8"));
  expect(manifest.name).toBe("newde-runtime");

  const hooks = JSON.parse(readFileSync(plugin.hooksPath, "utf8"));
  for (const event of [
    "PreToolUse",
    "PostToolUse",
    "UserPromptSubmit",
    "SessionStart",
    "SessionEnd",
    "Stop",
    "Notification",
  ]) {
    const entry = hooks.hooks[event][0].hooks[0];
    expect(entry.type).toBe("http");
    expect(entry.url).toBe(`http://127.0.0.1:12345/hook/${event}`);
    expect(entry.headers.Authorization).toBe("Bearer $NEWDE_HOOK_TOKEN");
    expect(entry.headers["X-Newde-Stream"]).toBe("$NEWDE_STREAM_ID");
    expect(entry.headers["X-Newde-Batch"]).toBe("$NEWDE_BATCH_ID");
    expect(entry.headers["X-Newde-Pane"]).toBe("$NEWDE_PANE");
    expect(entry.allowedEnvVars).toEqual(expect.arrayContaining([
      "NEWDE_HOOK_TOKEN",
      "NEWDE_STREAM_ID",
      "NEWDE_BATCH_ID",
      "NEWDE_PANE",
    ]));
  }
});

test("createElectronPlugin writes an AGENT_GUIDE.md the agent can Read on demand", () => {
  const projectDir = mkdtempSync(join(tmpdir(), "newde-project-"));
  const plugin = createElectronPlugin({ projectDir, hookUrl: "http://127.0.0.1:1/hook" });
  expect(plugin.agentGuidePath).toBe(join(plugin.pluginDir, "AGENT_GUIDE.md"));
  expect(existsSync(plugin.agentGuidePath)).toBe(true);
  const text = readFileSync(plugin.agentGuidePath, "utf8");
  // The reference catalog we pulled out of the system prompt should all
  // appear in the on-disk guide so trimming the prompt isn't a regression.
  expect(text).toContain("blocks");
  expect(text).toContain("discovered_from");
  expect(text).toContain("relates_to");
  expect(text).toContain("duplicates");
  expect(text).toContain("supersedes");
  expect(text).toContain("replies_to");
  expect(text).toContain("epic");
  expect(text).toContain("task");
  expect(text).toContain("subtask");
  expect(text).toContain("bug");
  expect(text).toContain("note");
});

test("createElectronPlugin is idempotent across calls", () => {
  const projectDir = mkdtempSync(join(tmpdir(), "newde-project-"));
  const first = createElectronPlugin({ projectDir, hookUrl: "http://127.0.0.1:1/hook" });
  const firstManifest = readFileSync(first.manifestPath, "utf8");
  const firstHooks = readFileSync(first.hooksPath, "utf8");

  const firstGuide = readFileSync(first.agentGuidePath, "utf8");

  const second = createElectronPlugin({ projectDir, hookUrl: "http://127.0.0.1:1/hook" });
  expect(second.pluginDir).toBe(first.pluginDir);
  expect(readFileSync(second.manifestPath, "utf8")).toBe(firstManifest);
  expect(readFileSync(second.hooksPath, "utf8")).toBe(firstHooks);
  expect(readFileSync(second.agentGuidePath, "utf8")).toBe(firstGuide);
});
