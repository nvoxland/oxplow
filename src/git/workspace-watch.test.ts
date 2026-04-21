import { afterEach, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import type { Logger } from "../core/logger.js";
import { WorkspaceWatcherRegistry, shouldIgnoreWorkspaceWatchPath } from "./workspace-watch.js";

const tempDirs: string[] = [];

afterEach(() => {
  while (tempDirs.length > 0) {
    rmSync(tempDirs.pop()!, { recursive: true, force: true });
  }
});

test("workspace watcher subscribers are stream-scoped", () => {
  const registry = new WorkspaceWatcherRegistry(noopLogger());
  const received: string[] = [];
  const unsubscribe = registry.subscribe((event) => {
    received.push(`${event.streamId}:${event.kind}:${event.path}`);
  }, "stream-a");
  registry.notify("stream-a", "created", "src/app.ts");
  registry.notify("stream-b", "updated", "src/other.ts");
  unsubscribe();
  registry.notify("stream-a", "deleted", "src/ignored.ts");
  expect(received).toEqual(["stream-a:created:src/app.ts"]);
});

test("workspace watcher emits events for filesystem changes", async () => {
  const root = mkdtempSync(join(tmpdir(), "newde-watch-"));
  tempDirs.push(root);
  mkdirSync(join(root, "src"), { recursive: true });
  const registry = new WorkspaceWatcherRegistry(noopLogger());
  const events: string[] = [];
  registry.subscribe((event) => {
    if (event.path) {
      events.push(`${event.kind}:${event.path}`);
    }
  }, "stream-a");
  registry.ensureWatching({
    id: "stream-a",
    title: "Stream A",
    summary: "",
    branch: "main",
    branch_ref: "refs/heads/main",
    branch_source: "local",
    worktree_path: root,
    created_at: "2024-01-01T00:00:00.000Z",
    updated_at: "2024-01-01T00:00:00.000Z",
    custom_prompt: null,
    panes: { working: "a:1.0", talking: "a:1.1" },
    resume: { working_session_id: "", talking_session_id: "" },
  });

  writeFileSync(join(root, "src", "watch-me.ts"), "export const value = 1;\n", "utf8");

  await waitFor(() => events.some((event) => event.endsWith(":src/watch-me.ts")));
  registry.dispose();
  expect(events.some((event) => event.endsWith(":src/watch-me.ts"))).toBe(true);
});

test("workspace watcher ignores internal runtime paths", () => {
  expect(shouldIgnoreWorkspaceWatchPath(".git/index")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath(".newde/logs/system.log")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath(".newde/worktrees/feature/src/app.ts")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath(".newde/state.db")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath(".newde/snapshots/objects/ab/cd")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("src/app.ts")).toBe(false);
});

test("workspace watcher ignores editor/agent temp files", () => {
  expect(shouldIgnoreWorkspaceWatchPath(".context/foo.md.tmp.18726.1776752930633")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("foo.md~")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath(".foo.swp")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("src/foo.swo")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("src/foo.swx")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("#foo#")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("dir/foo.tmp")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath(".context/foo.md")).toBe(false);
  expect(shouldIgnoreWorkspaceWatchPath("src/index.ts")).toBe(false);
});

test("workspace watcher ignores additional directory names from extras", () => {
  expect(shouldIgnoreWorkspaceWatchPath("generated/api.ts", ["generated"])).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("packages/a/generated", ["generated"])).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("src/not-generated/foo.ts", ["generated"])).toBe(false);
  // Without extras, the path is kept.
  expect(shouldIgnoreWorkspaceWatchPath("generated/api.ts")).toBe(false);
});

test("workspace watcher ignores heavy build/cache directories", () => {
  expect(shouldIgnoreWorkspaceWatchPath("node_modules")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("node_modules/react/index.js")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("packages/a/node_modules/foo")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("dist")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("build/out.js")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("target/debug/app")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath(".next/cache")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath(".turbo")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath(".cache")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("coverage/lcov.info")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("__pycache__")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath(".venv/bin/python")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath(".idea/workspace.xml")).toBe(true);
  expect(shouldIgnoreWorkspaceWatchPath("src/app.ts")).toBe(false);
  expect(shouldIgnoreWorkspaceWatchPath("src/__pycache_not__")).toBe(false);
});

function noopLogger(): Logger {
  return {
    path: "",
    debug() {},
    info() {},
    warn() {},
    error() {},
    child() {
      return noopLogger();
    },
  };
}

async function waitFor(predicate: () => boolean, timeoutMs = 2000): Promise<void> {
  const start = Date.now();
  while (!predicate()) {
    if (Date.now() - start > timeoutMs) {
      throw new Error("timed out waiting for filesystem event");
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}
