import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { WorkItemCommitStore } from "./work-item-commit-store.js";
import { WorkItemStore } from "./work-item-store.js";
import { ThreadStore } from "./thread-store.js";
import { StreamStore } from "./stream-store.js";

function seed() {
  const dir = mkdtempSync(join(tmpdir(), "newde-wic-"));
  const streamStore = new StreamStore(dir);
  const stream = streamStore.create({
    title: "S",
    branch: "main",
    worktreePath: dir,
    projectBase: "demo",
  });
  const threadStore = new ThreadStore(dir);
  const state = threadStore.ensureStream(stream);
  const threadId = state.threads[0]!.id;
  const workItems = new WorkItemStore(dir);
  const junction = new WorkItemCommitStore(dir);
  return { dir, threadId, workItems, junction };
}

describe("WorkItemCommitStore", () => {
  test("insert + listShasForItem + listItemsForSha roundtrip", () => {
    const h = seed();
    const a = h.workItems.createItem({ threadId: h.threadId, kind: "task", title: "A", createdBy: "user", actorId: "t" });
    const b = h.workItems.createItem({ threadId: h.threadId, kind: "task", title: "B", createdBy: "user", actorId: "t" });
    h.junction.insert(a.id, "sha1", "2024-01-01T00:00:00Z");
    h.junction.insert(b.id, "sha1", "2024-01-01T00:00:00Z");
    h.junction.insert(a.id, "sha2", "2024-01-02T00:00:00Z");

    const aShas = h.junction.listShasForItem(a.id).map((r) => r.sha);
    expect(aShas.sort()).toEqual(["sha1", "sha2"]);
    const sha1Items = h.junction.listItemsForSha("sha1").map((r) => r.work_item_id);
    expect(sha1Items.sort()).toEqual([a.id, b.id].sort());
  });

  test("insert is idempotent on duplicate (work_item_id, sha)", () => {
    const h = seed();
    const a = h.workItems.createItem({ threadId: h.threadId, kind: "task", title: "A", createdBy: "user", actorId: "t" });
    h.junction.insert(a.id, "sha-x");
    h.junction.insert(a.id, "sha-x");
    expect(h.junction.listShasForItem(a.id)).toHaveLength(1);
  });
});
