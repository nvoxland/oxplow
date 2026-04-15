import { afterEach, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
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
  expect(current?.resume.working_session_id).toBe("");
});

test("loadAll backfills defaults for legacy stream files", () => {
  const projectDir = mkProjectDir();
  const streamsDir = join(projectDir, ".newde", "streams");
  const legacyBody = [
    `id: "s-legacy"`,
    `title: "Legacy"`,
    `summary: ""`,
    `branch: "main"`,
    `created_at: "2024-01-01T00:00:00.000Z"`,
    `updated_at: "2024-01-01T00:00:00.000Z"`,
    `panes:`,
    `  working: "session:working"`,
    `  talking: "session:talking"`,
    ``,
  ].join("\n");
  writeFileSync(join(streamsDir, "s-legacy.yml"), legacyBody, "utf8");

  const store = new StreamStore(projectDir);
  const stream = store.get("s-legacy");
  expect(stream?.branch_ref).toBe("refs/heads/main");
  expect(stream?.branch_source).toBe("local");
  expect(stream?.worktree_path).toBe(projectDir);
  expect(stream?.resume.talking_session_id).toBe("");
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

function mkProjectDir(): string {
  const dir = mkdtempSync(join(tmpdir(), "newde-stream-store-"));
  tempDirs.push(dir);
  const streamsDir = join(dir, ".newde", "streams");
  mkdirSync(streamsDir, { recursive: true });
  return dir;
}
