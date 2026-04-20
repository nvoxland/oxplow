import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BatchStore } from "./batch-store.js";
import { WorkItemStore } from "./work-item-store.js";
import type { Stream } from "./stream-store.js";

function seedBatch() {
  const dir = mkdtempSync(join(tmpdir(), "newde-work-items-"));
  const batchStore = new BatchStore(dir);
  const stream: Stream = {
    id: "s-1",
    title: "Demo",
    summary: "",
    branch: "main",
    branch_ref: "refs/heads/main",
    branch_source: "local",
    worktree_path: "/tmp/demo",
    created_at: "2024-01-01T00:00:00.000Z",
    updated_at: "2024-01-01T00:00:00.000Z",
    panes: { working: "newde-demo:working-s-1", talking: "newde-demo:talking-s-1" },
    resume: { working_session_id: "", talking_session_id: "" },
  };
  const state = batchStore.ensureStream(stream);
  const batchId = state.batches[0]!.id;
  const workItems = new WorkItemStore(dir);
  return { workItems, batchId };
}

describe("WorkItemStore acceptance_criteria", () => {
  test("createItem persists acceptance criteria and they roundtrip", () => {
    const { workItems, batchId } = seedBatch();
    const item = workItems.createItem({
      batchId,
      kind: "task",
      title: "Write login form",
      acceptanceCriteria: "- email + password inputs\n- submit posts to /login\n- shows error on 401",
      createdBy: "agent",
      actorId: "mcp",
    });
    expect(item.acceptance_criteria).toContain("email + password");
    const fetched = workItems.getItem(batchId, item.id);
    expect(fetched?.acceptance_criteria).toBe(item.acceptance_criteria);
  });

  test("createItem appends to the bottom of the list (regression: new items must sort after existing)", () => {
    const { workItems, batchId } = seedBatch();
    const first = workItems.createItem({ batchId, kind: "task", title: "first", createdBy: "user", actorId: "ui" });
    const second = workItems.createItem({ batchId, kind: "task", title: "second", createdBy: "user", actorId: "ui" });
    const third = workItems.createItem({ batchId, kind: "task", title: "third", createdBy: "user", actorId: "ui" });
    expect(second.sort_index).toBeGreaterThan(first.sort_index);
    expect(third.sort_index).toBeGreaterThan(second.sort_index);
    // The MAX+1 rule keeps holding after a rename / status change — a new
    // item still lands strictly past the existing maximum rather than sliding
    // into a gap.
    workItems.updateItem({ batchId, itemId: first.id, status: "done", actorKind: "user", actorId: "ui" });
    const fourth = workItems.createItem({ batchId, kind: "task", title: "fourth", createdBy: "user", actorId: "ui" });
    expect(fourth.sort_index).toBeGreaterThan(third.sort_index);
  });

  test("updateItem with acceptanceCriteria='' clears the field", () => {
    const { workItems, batchId } = seedBatch();
    const item = workItems.createItem({
      batchId,
      kind: "task",
      title: "X",
      acceptanceCriteria: "keep this",
      createdBy: "agent",
      actorId: "mcp",
    });
    workItems.updateItem({
      batchId,
      itemId: item.id,
      acceptanceCriteria: "",
      actorKind: "agent",
      actorId: "mcp",
    });
    expect(workItems.getItem(batchId, item.id)?.acceptance_criteria).toBeNull();
  });

  test("getItemDetail returns incoming + outgoing links + recent events", () => {
    const { workItems, batchId } = seedBatch();
    const parent = workItems.createItem({ batchId, kind: "epic", title: "Parent", createdBy: "agent", actorId: "mcp" });
    const child = workItems.createItem({ batchId, kind: "task", title: "Child", createdBy: "agent", actorId: "mcp" });
    workItems.linkItems(batchId, child.id, parent.id, "supersedes");
    workItems.addNote(batchId, child.id, "made progress", "agent", "mcp");
    const detail = workItems.getItemDetail(batchId, child.id);
    expect(detail).not.toBeNull();
    expect(detail!.outgoing).toHaveLength(1);
    expect(detail!.outgoing[0]!.link_type).toBe("supersedes");
    expect(detail!.recentEvents.length).toBeGreaterThan(0);
  });
});

describe("WorkItemStore.readWorkOptions", () => {
  test("returns empty when no ready items exist", () => {
    const { workItems, batchId } = seedBatch();
    const result = workItems.readWorkOptions(batchId);
    expect(result.mode).toBe("empty");
  });

  test("returns epic unit when highest-priority ready item is an epic", () => {
    const { workItems, batchId } = seedBatch();
    const epic = workItems.createItem({ batchId, kind: "epic", title: "Big Feature", createdBy: "user", actorId: "ui" });
    const child1 = workItems.createItem({ batchId, kind: "task", title: "Task A", parentId: epic.id, createdBy: "user", actorId: "ui" });
    const child2 = workItems.createItem({ batchId, kind: "task", title: "Task B", parentId: epic.id, createdBy: "user", actorId: "ui" });
    const result = workItems.readWorkOptions(batchId);
    expect(result.mode).toBe("epic");
    if (result.mode !== "epic") return;
    expect(result.epic.id).toBe(epic.id);
    expect(result.children).toHaveLength(2);
    const childIds = result.children.map((c) => c.item.id);
    expect(childIds).toContain(child1.id);
    expect(childIds).toContain(child2.id);
  });

  test("epic mode: only includes ready descendants, excludes blocked/done children", () => {
    const { workItems, batchId } = seedBatch();
    const epic = workItems.createItem({ batchId, kind: "epic", title: "Epic", createdBy: "user", actorId: "ui" });
    const ready = workItems.createItem({ batchId, kind: "task", title: "Ready", parentId: epic.id, createdBy: "user", actorId: "ui" });
    const done = workItems.createItem({ batchId, kind: "task", title: "Done", parentId: epic.id, createdBy: "user", actorId: "ui", status: "done" });
    const result = workItems.readWorkOptions(batchId);
    expect(result.mode).toBe("epic");
    if (result.mode !== "epic") return;
    const childIds = result.children.map((c) => c.item.id);
    expect(childIds).toContain(ready.id);
    expect(childIds).not.toContain(done.id);
  });

  test("epic mode: blocked items (via blocks link) excluded from children", () => {
    const { workItems, batchId } = seedBatch();
    const epic = workItems.createItem({ batchId, kind: "epic", title: "Epic", createdBy: "user", actorId: "ui" });
    const blocker = workItems.createItem({ batchId, kind: "task", title: "Blocker", parentId: epic.id, createdBy: "user", actorId: "ui" });
    const blocked = workItems.createItem({ batchId, kind: "task", title: "Blocked", parentId: epic.id, createdBy: "user", actorId: "ui" });
    workItems.linkItems(batchId, blocker.id, blocked.id, "blocks");
    const result = workItems.readWorkOptions(batchId);
    expect(result.mode).toBe("epic");
    if (result.mode !== "epic") return;
    // blocked child excluded — it has an unresolved blocker
    const childIds = result.children.map((c) => c.item.id);
    expect(childIds).toContain(blocker.id);
    expect(childIds).not.toContain(blocked.id);
  });

  test("epic mode: children include link edges inline", () => {
    const { workItems, batchId } = seedBatch();
    const epic = workItems.createItem({ batchId, kind: "epic", title: "Epic", createdBy: "user", actorId: "ui" });
    const task1 = workItems.createItem({ batchId, kind: "task", title: "T1", parentId: epic.id, createdBy: "user", actorId: "ui" });
    const task2 = workItems.createItem({ batchId, kind: "task", title: "T2", parentId: epic.id, createdBy: "user", actorId: "ui" });
    workItems.linkItems(batchId, task1.id, task2.id, "relates_to");
    const result = workItems.readWorkOptions(batchId);
    expect(result.mode).toBe("epic");
    if (result.mode !== "epic") return;
    const t1 = result.children.find((c) => c.item.id === task1.id)!;
    expect(t1.outgoing).toHaveLength(1);
    expect(t1.outgoing[0]!.link_type).toBe("relates_to");
  });

  test("returns standalone unit when head is not an epic", () => {
    const { workItems, batchId } = seedBatch();
    const t1 = workItems.createItem({ batchId, kind: "task", title: "T1", createdBy: "user", actorId: "ui" });
    const t2 = workItems.createItem({ batchId, kind: "task", title: "T2", createdBy: "user", actorId: "ui" });
    const result = workItems.readWorkOptions(batchId);
    expect(result.mode).toBe("standalone");
    if (result.mode !== "standalone") return;
    expect(result.items).toHaveLength(2);
    const ids = result.items.map((i) => i.item.id);
    expect(ids).toContain(t1.id);
    expect(ids).toContain(t2.id);
  });

  test("standalone mode: epics are excluded from the items list", () => {
    const { workItems, batchId } = seedBatch();
    const task = workItems.createItem({ batchId, kind: "task", title: "Task", createdBy: "user", actorId: "ui" });
    // Epic created after task so task is still the head (lower sort_index)
    workItems.createItem({ batchId, kind: "epic", title: "Epic", createdBy: "user", actorId: "ui" });
    const result = workItems.readWorkOptions(batchId);
    expect(result.mode).toBe("standalone");
    if (result.mode !== "standalone") return;
    const ids = result.items.map((i) => i.item.id);
    expect(ids).toContain(task.id);
    // epic not in the standalone list
    expect(result.items.every((i) => i.item.kind !== "epic")).toBe(true);
  });

  test("standalone mode: items include link edges inline", () => {
    const { workItems, batchId } = seedBatch();
    const t1 = workItems.createItem({ batchId, kind: "task", title: "T1", createdBy: "user", actorId: "ui" });
    const t2 = workItems.createItem({ batchId, kind: "task", title: "T2", createdBy: "user", actorId: "ui" });
    workItems.linkItems(batchId, t1.id, t2.id, "discovered_from");
    const result = workItems.readWorkOptions(batchId);
    expect(result.mode).toBe("standalone");
    if (result.mode !== "standalone") return;
    const item1 = result.items.find((i) => i.item.id === t1.id)!;
    expect(item1.outgoing[0]!.link_type).toBe("discovered_from");
    const item2 = result.items.find((i) => i.item.id === t2.id)!;
    expect(item2.incoming[0]!.link_type).toBe("discovered_from");
  });

  test("epic mode: transitively ready grandchildren are included", () => {
    const { workItems, batchId } = seedBatch();
    const epic = workItems.createItem({ batchId, kind: "epic", title: "Epic", createdBy: "user", actorId: "ui" });
    const task = workItems.createItem({ batchId, kind: "task", title: "Task", parentId: epic.id, createdBy: "user", actorId: "ui" });
    const subtask = workItems.createItem({ batchId, kind: "subtask", title: "Subtask", parentId: task.id, createdBy: "user", actorId: "ui" });
    const result = workItems.readWorkOptions(batchId);
    expect(result.mode).toBe("epic");
    if (result.mode !== "epic") return;
    const ids = result.children.map((c) => c.item.id);
    expect(ids).toContain(task.id);
    expect(ids).toContain(subtask.id);
  });

  test("beforeSortIndex cuts off items at or beyond a commit/wait boundary", () => {
    const { workItems, batchId } = seedBatch();
    const t1 = workItems.createItem({ batchId, kind: "task", title: "Before", createdBy: "user", actorId: "ui" });
    const t2 = workItems.createItem({ batchId, kind: "task", title: "After", createdBy: "user", actorId: "ui" });
    // t1 has sort_index 0, t2 has sort_index 1; commit point sits at index 1
    const cutoff = t2.sort_index;
    const result = workItems.readWorkOptions(batchId, cutoff);
    expect(result.mode).toBe("standalone");
    if (result.mode !== "standalone") return;
    const ids = result.items.map((i) => i.item.id);
    expect(ids).toContain(t1.id);
    expect(ids).not.toContain(t2.id);
  });

  test("beforeSortIndex with no items before cutoff returns empty", () => {
    const { workItems, batchId } = seedBatch();
    workItems.createItem({ batchId, kind: "task", title: "After boundary", createdBy: "user", actorId: "ui" });
    // cutoff at sort_index 0 means nothing qualifies (items need sort_index < 0)
    const result = workItems.readWorkOptions(batchId, 0);
    expect(result.mode).toBe("empty");
  });
});
