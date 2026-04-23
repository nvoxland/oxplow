import { describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ThreadStore } from "../persistence/thread-store.js";
import { SnapshotStore } from "../persistence/snapshot-store.js";
import { StreamStore } from "../persistence/stream-store.js";
import { WorkItemStore } from "../persistence/work-item-store.js";
import { WorkItemEffortStore } from "../persistence/work-item-effort-store.js";
import { computeLocalBlame } from "./local-blame.js";
import type { BlameLine } from "../git/git.js";

function seed() {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-local-blame-"));
  const streamStore = new StreamStore(dir);
  const stream = streamStore.create({
    title: "Demo",
    branch: "main",
    worktreePath: dir,
    projectBase: "demo",
  });
  const threadStore = new ThreadStore(dir);
  const state = threadStore.ensureStream(stream);
  const threadId = state.threads[0]!.id;
  const workItems = new WorkItemStore(dir);
  const snapshots = new SnapshotStore(dir);
  const efforts = new WorkItemEffortStore(dir);
  return { dir, workItems, snapshots, efforts, threadId, streamId: stream.id };
}

function flushWith(dir: string, snapshots: SnapshotStore, streamId: string, path: string, content: string): string {
  writeFileSync(join(dir, path), content);
  const result = snapshots.flushSnapshot({
    source: "task-start",
    streamId,
    worktreePath: dir,
    dirtyPaths: [path],
  });
  return result.id;
}

describe("computeLocalBlame", () => {
  test("attributes lines to the effort that last changed them, newest wins", () => {
    const { dir, workItems, snapshots, efforts, threadId, streamId } = seed();
    const path = "hello.txt";
    // Initial empty baseline (for snapshot dedup to work).
    writeFileSync(join(dir, path), "");
    const baseline = snapshots.flushSnapshot({
      source: "task-start",
      streamId,
      worktreePath: dir,
      dirtyPaths: [path],
    });

    // Effort A: creates lines ["a1", "a2", "a3"]
    const itemA = workItems.createItem({
      threadId, kind: "task", title: "A", createdBy: "user", actorId: "t",
    });
    const aStart = baseline.id;
    const aEnd = flushWith(dir, snapshots, streamId, path, "a1\na2\na3\n");
    const effA = efforts.openEffort({ workItemId: itemA.id, startSnapshotId: aStart });
    efforts.recordEffortFile(effA.id, path);
    // Close effort A with ended_at A
    Bun.sleepSync(3);
    efforts.closeEffort({ workItemId: itemA.id, endSnapshotId: aEnd });

    // Effort B: modifies middle line -> ["a1", "b2", "a3"]
    const itemB = workItems.createItem({
      threadId, kind: "task", title: "B", createdBy: "user", actorId: "t",
    });
    const bStart = aEnd;
    const bEnd = flushWith(dir, snapshots, streamId, path, "a1\nb2\na3\n");
    const effB = efforts.openEffort({ workItemId: itemB.id, startSnapshotId: bStart });
    efforts.recordEffortFile(effB.id, path);
    Bun.sleepSync(3);
    efforts.closeEffort({ workItemId: itemB.id, endSnapshotId: bEnd });

    // Effort C: appends a new line -> ["a1", "b2", "a3", "c4"]
    const itemC = workItems.createItem({
      threadId, kind: "task", title: "C", createdBy: "user", actorId: "t",
    });
    const cStart = bEnd;
    const cEnd = flushWith(dir, snapshots, streamId, path, "a1\nb2\na3\nc4\n");
    const effC = efforts.openEffort({ workItemId: itemC.id, startSnapshotId: cStart });
    efforts.recordEffortFile(effC.id, path);
    Bun.sleepSync(3);
    efforts.closeEffort({ workItemId: itemC.id, endSnapshotId: cEnd });

    const diskText = "a1\nb2\na3\nc4\n";
    const blame = computeLocalBlame({
      effortStore: efforts,
      snapshotStore: snapshots,
      path,
      diskText,
      gitBlame: () => [],
    });

    // 4 lines.
    expect(blame).toHaveLength(4);
    // a1 — first added by effort A, never touched again.
    expect(blame[0]!.source).toBe("local");
    expect(blame[0]!.workItem?.title).toBe("A");
    // b2 — last touched by B.
    expect(blame[1]!.source).toBe("local");
    expect(blame[1]!.workItem?.title).toBe("B");
    // a3 — added by A.
    expect(blame[2]!.source).toBe("local");
    expect(blame[2]!.workItem?.title).toBe("A");
    // c4 — added by C.
    expect(blame[3]!.source).toBe("local");
    expect(blame[3]!.workItem?.title).toBe("C");
  });

  test("disk edits after last effort surface as uncommitted", () => {
    const { dir, workItems, snapshots, efforts, threadId, streamId } = seed();
    const path = "hello.txt";
    writeFileSync(join(dir, path), "");
    const baseline = snapshots.flushSnapshot({
      source: "task-start", streamId, worktreePath: dir, dirtyPaths: [path],
    });

    const item = workItems.createItem({
      threadId, kind: "task", title: "A", createdBy: "user", actorId: "t",
    });
    const end = flushWith(dir, snapshots, streamId, path, "a1\na2\n");
    const eff = efforts.openEffort({ workItemId: item.id, startSnapshotId: baseline.id });
    efforts.recordEffortFile(eff.id, path);
    Bun.sleepSync(3);
    efforts.closeEffort({ workItemId: item.id, endSnapshotId: end });

    // Disk has one extra line not in any snapshot.
    const diskText = "a1\na2\nEDITED\n";
    const blame = computeLocalBlame({
      effortStore: efforts,
      snapshotStore: snapshots,
      path,
      diskText,
      gitBlame: () => [],
    });
    expect(blame).toHaveLength(3);
    expect(blame[0]!.source).toBe("local");
    expect(blame[1]!.source).toBe("local");
    expect(blame[2]!.source).toBe("uncommitted");
  });

  test("falls back to git blame when local walk cannot cover a line", () => {
    const { dir, snapshots, efforts, streamId } = seed();
    const path = "hello.txt";
    // Simulate a file with no effort history at all.
    const diskText = "old1\nold2\n";
    writeFileSync(join(dir, path), diskText);
    snapshots.flushSnapshot({
      source: "task-start", streamId, worktreePath: dir, dirtyPaths: [path],
    });

    const gitBlameLines: BlameLine[] = [
      { line: 1, sha: "abc123abc123abc123abc123abc123abc123abc1", author: "Alice", authorMail: "a@x", authorTime: 1700000000, summary: "initial" },
      { line: 2, sha: "abc123abc123abc123abc123abc123abc123abc1", author: "Alice", authorMail: "a@x", authorTime: 1700000000, summary: "initial" },
    ];

    const blame = computeLocalBlame({
      effortStore: efforts,
      snapshotStore: snapshots,
      path,
      diskText,
      gitBlame: () => gitBlameLines,
    });
    expect(blame).toHaveLength(2);
    expect(blame[0]!.source).toBe("git");
    expect(blame[0]!.git?.sha).toBe(gitBlameLines[0]!.sha);
    expect(blame[1]!.source).toBe("git");
  });

  test("skips efforts whose snapshots are pruned (snapshot missing)", () => {
    const { dir, workItems, snapshots, efforts, threadId, streamId } = seed();
    const path = "hello.txt";
    writeFileSync(join(dir, path), "");
    const baseline = snapshots.flushSnapshot({
      source: "task-start", streamId, worktreePath: dir, dirtyPaths: [path],
    });
    const item = workItems.createItem({
      threadId, kind: "task", title: "A", createdBy: "user", actorId: "t",
    });
    const eff = efforts.openEffort({ workItemId: item.id, startSnapshotId: baseline.id });
    efforts.recordEffortFile(eff.id, path);
    // Close without an end_snapshot_id (simulates pruning). The effort should
    // be skipped gracefully.
    Bun.sleepSync(3);
    efforts.closeEffort({ workItemId: item.id, endSnapshotId: null });

    const diskText = "a1\n";
    writeFileSync(join(dir, path), diskText);
    const gitBlameLines: BlameLine[] = [
      { line: 1, sha: "def456def456def456def456def456def456def4", author: "Bob", authorMail: "b@x", authorTime: 1700000000, summary: "x" },
    ];
    const blame = computeLocalBlame({
      effortStore: efforts,
      snapshotStore: snapshots,
      path,
      diskText,
      gitBlame: () => gitBlameLines,
    });
    expect(blame).toHaveLength(1);
    // Falls back to git since the effort's end snapshot is null.
    expect(blame[0]!.source).toBe("git");
  });
});
