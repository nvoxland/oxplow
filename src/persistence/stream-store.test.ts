import { afterEach, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { StreamStore } from "./stream-store.js";

const tempDirs: string[] = [];

afterEach(() => {
  while (tempDirs.length > 0) {
    const dir = tempDirs.pop();
    if (dir) rmSync(dir, { recursive: true, force: true });
  }
});

test("create persists extended stream metadata and current stream selection", () => {
  const projectDir = mkProjectDir();
  const store = new StreamStore(projectDir);

  const stream = store.create({
    title: "Main",
    branch: "main",
    branchRef: "refs/heads/main",
    branchSource: "local",
    worktreePath: projectDir,
    projectBase: "proj",
  });
  store.setCurrentStreamId(stream.id);

  const reloaded = new StreamStore(projectDir);
  const current = reloaded.getCurrent();
  expect(current?.id).toBe(stream.id);
  expect(current?.branch_ref).toBe("refs/heads/main");
  expect(current?.worktree_path).toBe(projectDir);
  expect(current?.panes.working).toContain(`working-${stream.id}`);
});

test("list persists multiple streams in creation order", () => {
  const projectDir = mkProjectDir();
  const store = new StreamStore(projectDir);
  const first = store.create({
    title: "First",
    branch: "main",
    worktreePath: projectDir,
    projectBase: "proj",
  });
  const second = store.create({
    title: "Second",
    branch: "feature",
    worktreePath: join(projectDir, "feature"),
    projectBase: "proj",
  });

  const reloaded = new StreamStore(projectDir);
  expect(reloaded.list().map((stream) => stream.id)).toEqual([first.id, second.id]);
  expect(reloaded.findByBranch("feature")?.id).toBe(second.id);
});

test("update persists renamed title", () => {
  const projectDir = mkProjectDir();
  const store = new StreamStore(projectDir);
  const created = store.create({
    title: "Old name",
    branch: "main",
    worktreePath: projectDir,
    projectBase: "proj",
  });

  store.update(created.id, (stream) => ({ ...stream, title: "New name" }));

  const reloaded = new StreamStore(projectDir);
  expect(reloaded.get(created.id)?.title).toBe("New name");
});

test("create defaults kind to 'worktree' and honours explicit 'primary'", () => {
  const projectDir = mkProjectDir();
  const store = new StreamStore(projectDir);

  const worktree = store.create({
    title: "wt",
    branch: "feat",
    worktreePath: join(projectDir, "feat"),
    projectBase: "proj",
  });
  const primary = store.create({
    title: "proj",
    branch: "main",
    worktreePath: projectDir,
    projectBase: "proj",
    kind: "primary",
  });

  expect(store.get(worktree.id)?.kind).toBe("worktree");
  expect(store.get(primary.id)?.kind).toBe("primary");
});

test("findPrimary returns null when no primary exists and the primary row once created", () => {
  const projectDir = mkProjectDir();
  const store = new StreamStore(projectDir);
  expect(store.findPrimary()).toBeUndefined();

  store.create({
    title: "feat",
    branch: "feat",
    worktreePath: join(projectDir, "feat"),
    projectBase: "proj",
  });
  expect(store.findPrimary()).toBeUndefined();

  const primary = store.create({
    title: "proj",
    branch: "main",
    worktreePath: projectDir,
    projectBase: "proj",
    kind: "primary",
  });
  expect(store.findPrimary()?.id).toBe(primary.id);

  store.create({
    title: "other",
    branch: "other",
    worktreePath: join(projectDir, "other"),
    projectBase: "proj",
  });
  expect(store.findPrimary()?.id).toBe(primary.id);
});

test("setStreamBranch updates branch + branch_ref and emits stream.changed", () => {
  const projectDir = mkProjectDir();
  const store = new StreamStore(projectDir);
  const primary = store.create({
    title: "proj",
    branch: "main",
    worktreePath: projectDir,
    projectBase: "proj",
    kind: "primary",
  });
  const other = store.create({
    title: "feat",
    branch: "feat",
    worktreePath: join(projectDir, "feat"),
    projectBase: "proj",
  });

  const events: unknown[] = [];
  store.subscribe((event) => events.push(event));

  const updated = store.setStreamBranch(primary.id, "next", "refs/heads/next");
  expect(updated.branch).toBe("next");
  expect(updated.branch_ref).toBe("refs/heads/next");

  const reloaded = new StreamStore(projectDir);
  expect(reloaded.get(primary.id)?.branch).toBe("next");
  expect(reloaded.get(primary.id)?.branch_ref).toBe("refs/heads/next");
  // Sibling untouched.
  expect(reloaded.get(other.id)?.branch).toBe("feat");
  expect(events).toEqual([{ kind: "branch-changed", streamId: primary.id }]);
});

test("deleteStream refuses to remove the primary stream", () => {
  const projectDir = mkProjectDir();
  const store = new StreamStore(projectDir);
  const primary = store.create({
    title: "proj",
    branch: "main",
    worktreePath: projectDir,
    projectBase: "proj",
    kind: "primary",
  });
  expect(() => store.deleteStream(primary.id)).toThrow(/primary/);
});

function mkProjectDir(): string {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-stream-store-"));
  tempDirs.push(dir);
  return dir;
}
