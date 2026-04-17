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

test("createElectronPlugin is idempotent across calls", () => {
  const projectDir = mkdtempSync(join(tmpdir(), "newde-project-"));
  const first = createElectronPlugin({ projectDir, hookUrl: "http://127.0.0.1:1/hook" });
  const firstManifest = readFileSync(first.manifestPath, "utf8");
  const firstHooks = readFileSync(first.hooksPath, "utf8");

  const second = createElectronPlugin({ projectDir, hookUrl: "http://127.0.0.1:1/hook" });
  expect(second.pluginDir).toBe(first.pluginDir);
  expect(readFileSync(second.manifestPath, "utf8")).toBe(firstManifest);
  expect(readFileSync(second.hooksPath, "utf8")).toBe(firstHooks);
});
