import { afterEach, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import type { Logger } from "./logger.js";
import { WorkspaceWatcherRegistry } from "./workspace-watch.js";

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
    panes: { working: "a:1.0", talking: "a:1.1" },
    resume: { working_session_id: "", talking_session_id: "" },
  });

  writeFileSync(join(root, "src", "watch-me.ts"), "export const value = 1;\n", "utf8");

  await waitFor(() => events.some((event) => event.endsWith(":src/watch-me.ts")));
  registry.dispose();
  expect(events.some((event) => event.endsWith(":src/watch-me.ts"))).toBe(true);
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
