import { afterEach, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import type { Logger } from "../core/logger.js";
import { GitRefsWatcherRegistry } from "./git-refs-watch.js";

const tempDirs: string[] = [];

afterEach(() => {
  while (tempDirs.length > 0) {
    rmSync(tempDirs.pop()!, { recursive: true, force: true });
  }
});

test("git refs watcher fires after a commit inside a secondary worktree", async () => {
  const repoDir = mkdtempSync(join(tmpdir(), "newde-refs-repo-"));
  tempDirs.push(repoDir);
  execFileSync("git", ["init", "-b", "main", repoDir], { stdio: "ignore" });
  execFileSync("git", ["-C", repoDir, "config", "user.name", "Test"], { stdio: "ignore" });
  execFileSync("git", ["-C", repoDir, "config", "user.email", "t@e.x"], { stdio: "ignore" });
  execFileSync("git", ["-C", repoDir, "commit", "--allow-empty", "-m", "init"], { stdio: "ignore" });

  const worktreeDir = mkdtempSync(join(tmpdir(), "newde-refs-wt-"));
  tempDirs.push(worktreeDir);
  rmSync(worktreeDir, { recursive: true, force: true });
  execFileSync("git", ["-C", repoDir, "worktree", "add", worktreeDir, "-b", "feature"], { stdio: "ignore" });

  const registry = new GitRefsWatcherRegistry(noopLogger());
  const changes: string[] = [];
  registry.subscribe((c) => changes.push(c.streamId));
  registry.ensureWatching({
    id: "s1",
    title: "S1",
    summary: "",
    branch: "feature",
    branch_ref: "refs/heads/feature",
    branch_source: "local",
    worktree_path: worktreeDir,
    created_at: "",
    updated_at: "",
    panes: { working: "a:1.0", talking: "a:1.1" },
    archived_at: null,
  } as any);

  execFileSync("git", ["-C", worktreeDir, "commit", "--allow-empty", "-m", "x"], { stdio: "ignore" });

  await waitFor(() => changes.length > 0, 2000);
  registry.dispose();
  expect(changes.length).toBeGreaterThan(0);
});

async function waitFor(check: () => boolean, timeoutMs: number): Promise<void> {
  const start = Date.now();
  while (!check()) {
    if (Date.now() - start > timeoutMs) return;
    await new Promise((r) => setTimeout(r, 25));
  }
}

function noopLogger(): Logger {
  const fn = () => {};
  const logger: any = { debug: fn, info: fn, warn: fn, error: fn, child: () => logger };
  return logger as Logger;
}
