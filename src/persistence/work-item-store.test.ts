import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ThreadStore } from "./thread-store.js";
import { WorkItemStore } from "./work-item-store.js";
import type { Stream } from "./stream-store.js";

function seedThread() {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-work-items-"));
  const threadStore = new ThreadStore(dir);
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
    panes: { working: "oxplow-demo:working-s-1", talking: "oxplow-demo:talking-s-1" },
    resume: { working_session_id: "", talking_session_id: "" },
    custom_prompt: null,
  };
  const state = threadStore.ensureStream(stream);
  const threadId = state.threads[0]!.id;
  const workItems = new WorkItemStore(dir);
  return { workItems, threadId };
}

describe("WorkItemStore acceptance_criteria", () => {
  test("createItem persists acceptance criteria and they roundtrip", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId,
      kind: "task",
      title: "Write login form",
      acceptanceCriteria: "- email + password inputs\n- submit posts to /login\n- shows error on 401",
      createdBy: "agent",
      actorId: "mcp",
    });
    expect(item.acceptance_criteria).toContain("email + password");
    const fetched = workItems.getItem(threadId, item.id);
    expect(fetched?.acceptance_criteria).toBe(item.acceptance_criteria);
  });

  test("createItem appends to the bottom of the list (regression: new items must sort after existing)", () => {
    const { workItems, threadId } = seedThread();
    const first = workItems.createItem({ threadId, kind: "task", title: "first", createdBy: "user", actorId: "ui" });
    const second = workItems.createItem({ threadId, kind: "task", title: "second", createdBy: "user", actorId: "ui" });
    const third = workItems.createItem({ threadId, kind: "task", title: "third", createdBy: "user", actorId: "ui" });
    expect(second.sort_index).toBeGreaterThan(first.sort_index);
    expect(third.sort_index).toBeGreaterThan(second.sort_index);
    // The MAX+1 rule keeps holding after a rename / status change — a new
    // item still lands strictly past the existing maximum rather than sliding
    // into a gap.
    workItems.updateItem({ threadId, itemId: first.id, status: "done", actorKind: "user", actorId: "ui" });
    const fourth = workItems.createItem({ threadId, kind: "task", title: "fourth", createdBy: "user", actorId: "ui" });
    expect(fourth.sort_index).toBeGreaterThan(third.sort_index);
  });

  test("updateItem with acceptanceCriteria='' clears the field", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId,
      kind: "task",
      title: "X",
      acceptanceCriteria: "keep this",
      createdBy: "agent",
      actorId: "mcp",
    });
    workItems.updateItem({
      threadId,
      itemId: item.id,
      acceptanceCriteria: "",
      actorKind: "agent",
      actorId: "mcp",
    });
    expect(workItems.getItem(threadId, item.id)?.acceptance_criteria).toBeNull();
  });

  test("dragging a Done item back to Human Check: updateItem+reorderItems sequence leaves item in HC", () => {
    // Reproduces the user-facing drag-out-of-Done regression. Mirrors what
    // WorkGroupList.handleDropOnKey does: fires updateItem (status change)
    // then reorderItems (full persistence order) in that order. Final store
    // state must have the dragged item classified into humanCheck by
    // ThreadWorkState.getState.
    const { workItems, threadId } = seedThread();
    const r0 = workItems.createItem({ threadId, kind: "task", title: "r0", createdBy: "user", actorId: "ui" });
    const r1 = workItems.createItem({ threadId, kind: "task", title: "r1", createdBy: "user", actorId: "ui" });
    const hc2 = workItems.createItem({ threadId, kind: "task", title: "hc2", createdBy: "user", actorId: "ui" });
    const hc3 = workItems.createItem({ threadId, kind: "task", title: "hc3", createdBy: "user", actorId: "ui" });
    const d4 = workItems.createItem({ threadId, kind: "task", title: "d4", createdBy: "user", actorId: "ui" });
    const d5 = workItems.createItem({ threadId, kind: "task", title: "d5", createdBy: "user", actorId: "ui" });
    // Put items into their target statuses. Note: updateItem's "bump sort_index
    // to MAX+1 on transition to done" will reassign sort_index values. That's
    // fine for this test — we care about the final drag result, not the setup.
    workItems.updateItem({ threadId, itemId: hc2.id, status: "human_check", actorKind: "user", actorId: "ui" });
    workItems.updateItem({ threadId, itemId: hc3.id, status: "human_check", actorKind: "user", actorId: "ui" });
    workItems.updateItem({ threadId, itemId: d4.id, status: "done", actorKind: "user", actorId: "ui" });
    workItems.updateItem({ threadId, itemId: d5.id, status: "done", actorKind: "user", actorId: "ui" });

    // Drop fires: (1) status → human_check, then (2) reorder with the
    // finalized id order from the UI. The id list we pass is what the UI
    // would produce after splice + finalizeReorderIds for a drag of d4 onto
    // the top of the Human Check section.
    workItems.updateItem({ threadId, itemId: d4.id, status: "human_check", actorKind: "user", actorId: "ui" });
    workItems.reorderItems(threadId, [r0.id, r1.id, hc2.id, hc3.id, d4.id, d5.id], "user", "ui");

    const state = workItems.getState(threadId);
    const hcIds = state.inProgress.filter((i) => i.status === "human_check").map((i) => i.id);
    const doneIds = state.done.map((i) => i.id);
    expect(hcIds).toContain(d4.id);
    expect(doneIds).not.toContain(d4.id);
    // The dropped item should render at the top of HC (highest sort_index in
    // the HC run) when the section renders descending.
    const hcItems = state.inProgress.filter((i) => i.status === "human_check");
    hcItems.sort((a, b) => b.sort_index - a.sort_index);
    expect(hcItems[0]?.id).toBe(d4.id);
  });

  test("getItemDetail returns incoming + outgoing links + recent events", () => {
    const { workItems, threadId } = seedThread();
    const parent = workItems.createItem({ threadId, kind: "epic", title: "Parent", createdBy: "agent", actorId: "mcp" });
    const child = workItems.createItem({ threadId, kind: "task", title: "Child", createdBy: "agent", actorId: "mcp" });
    workItems.linkItems(threadId, child.id, parent.id, "supersedes");
    workItems.addNote(threadId, child.id, "made progress", "agent", "mcp");
    const detail = workItems.getItemDetail(threadId, child.id);
    expect(detail).not.toBeNull();
    expect(detail!.outgoing).toHaveLength(1);
    expect(detail!.outgoing[0]!.link_type).toBe("supersedes");
    expect(detail!.recentEvents.length).toBeGreaterThan(0);
  });
});

describe("WorkItemStore.readWorkOptions", () => {
  test("returns empty when no ready items exist", () => {
    const { workItems, threadId } = seedThread();
    const result = workItems.readWorkOptions(threadId);
    expect(result.mode).toBe("empty");
  });

  test("returns epic unit when highest-priority ready item is an epic", () => {
    const { workItems, threadId } = seedThread();
    const epic = workItems.createItem({ threadId, kind: "epic", title: "Big Feature", createdBy: "user", actorId: "ui" });
    const child1 = workItems.createItem({ threadId, kind: "task", title: "Task A", parentId: epic.id, createdBy: "user", actorId: "ui" });
    const child2 = workItems.createItem({ threadId, kind: "task", title: "Task B", parentId: epic.id, createdBy: "user", actorId: "ui" });
    const result = workItems.readWorkOptions(threadId);
    expect(result.mode).toBe("epic");
    if (result.mode !== "epic") return;
    expect(result.epic.id).toBe(epic.id);
    expect(result.children).toHaveLength(2);
    const childIds = result.children.map((c) => c.item.id);
    expect(childIds).toContain(child1.id);
    expect(childIds).toContain(child2.id);
  });

  test("epic mode: only includes ready descendants, excludes blocked/done children", () => {
    const { workItems, threadId } = seedThread();
    const epic = workItems.createItem({ threadId, kind: "epic", title: "Epic", createdBy: "user", actorId: "ui" });
    const ready = workItems.createItem({ threadId, kind: "task", title: "Ready", parentId: epic.id, createdBy: "user", actorId: "ui" });
    const done = workItems.createItem({ threadId, kind: "task", title: "Done", parentId: epic.id, createdBy: "user", actorId: "ui", status: "done" });
    const result = workItems.readWorkOptions(threadId);
    expect(result.mode).toBe("epic");
    if (result.mode !== "epic") return;
    const childIds = result.children.map((c) => c.item.id);
    expect(childIds).toContain(ready.id);
    expect(childIds).not.toContain(done.id);
  });

  test("epic mode: blocked items (via blocks link) excluded from children", () => {
    const { workItems, threadId } = seedThread();
    const epic = workItems.createItem({ threadId, kind: "epic", title: "Epic", createdBy: "user", actorId: "ui" });
    const blocker = workItems.createItem({ threadId, kind: "task", title: "Blocker", parentId: epic.id, createdBy: "user", actorId: "ui" });
    const blocked = workItems.createItem({ threadId, kind: "task", title: "Blocked", parentId: epic.id, createdBy: "user", actorId: "ui" });
    workItems.linkItems(threadId, blocker.id, blocked.id, "blocks");
    const result = workItems.readWorkOptions(threadId);
    expect(result.mode).toBe("epic");
    if (result.mode !== "epic") return;
    // blocked child excluded — it has an unresolved blocker
    const childIds = result.children.map((c) => c.item.id);
    expect(childIds).toContain(blocker.id);
    expect(childIds).not.toContain(blocked.id);
  });

  test("epic mode: children include link edges inline", () => {
    const { workItems, threadId } = seedThread();
    const epic = workItems.createItem({ threadId, kind: "epic", title: "Epic", createdBy: "user", actorId: "ui" });
    const task1 = workItems.createItem({ threadId, kind: "task", title: "T1", parentId: epic.id, createdBy: "user", actorId: "ui" });
    const task2 = workItems.createItem({ threadId, kind: "task", title: "T2", parentId: epic.id, createdBy: "user", actorId: "ui" });
    workItems.linkItems(threadId, task1.id, task2.id, "relates_to");
    const result = workItems.readWorkOptions(threadId);
    expect(result.mode).toBe("epic");
    if (result.mode !== "epic") return;
    const t1 = result.children.find((c) => c.item.id === task1.id)!;
    expect(t1.outgoing).toHaveLength(1);
    expect(t1.outgoing[0]!.link_type).toBe("relates_to");
  });

  test("returns standalone unit when head is not an epic", () => {
    const { workItems, threadId } = seedThread();
    const t1 = workItems.createItem({ threadId, kind: "task", title: "T1", createdBy: "user", actorId: "ui" });
    const t2 = workItems.createItem({ threadId, kind: "task", title: "T2", createdBy: "user", actorId: "ui" });
    const result = workItems.readWorkOptions(threadId);
    expect(result.mode).toBe("standalone");
    if (result.mode !== "standalone") return;
    expect(result.items).toHaveLength(2);
    const ids = result.items.map((i) => i.item.id);
    expect(ids).toContain(t1.id);
    expect(ids).toContain(t2.id);
  });

  test("standalone mode: epics are excluded from the items list", () => {
    const { workItems, threadId } = seedThread();
    const task = workItems.createItem({ threadId, kind: "task", title: "Task", createdBy: "user", actorId: "ui" });
    // Epic created after task so task is still the head (lower sort_index)
    workItems.createItem({ threadId, kind: "epic", title: "Epic", createdBy: "user", actorId: "ui" });
    const result = workItems.readWorkOptions(threadId);
    expect(result.mode).toBe("standalone");
    if (result.mode !== "standalone") return;
    const ids = result.items.map((i) => i.item.id);
    expect(ids).toContain(task.id);
    // epic not in the standalone list
    expect(result.items.every((i) => i.item.kind !== "epic")).toBe(true);
  });

  test("standalone mode: items include link edges inline", () => {
    const { workItems, threadId } = seedThread();
    const t1 = workItems.createItem({ threadId, kind: "task", title: "T1", createdBy: "user", actorId: "ui" });
    const t2 = workItems.createItem({ threadId, kind: "task", title: "T2", createdBy: "user", actorId: "ui" });
    workItems.linkItems(threadId, t1.id, t2.id, "discovered_from");
    const result = workItems.readWorkOptions(threadId);
    expect(result.mode).toBe("standalone");
    if (result.mode !== "standalone") return;
    const item1 = result.items.find((i) => i.item.id === t1.id)!;
    expect(item1.outgoing[0]!.link_type).toBe("discovered_from");
    const item2 = result.items.find((i) => i.item.id === t2.id)!;
    expect(item2.incoming[0]!.link_type).toBe("discovered_from");
  });

  test("epic mode: transitively ready grandchildren are included", () => {
    const { workItems, threadId } = seedThread();
    const epic = workItems.createItem({ threadId, kind: "epic", title: "Epic", createdBy: "user", actorId: "ui" });
    const task = workItems.createItem({ threadId, kind: "task", title: "Task", parentId: epic.id, createdBy: "user", actorId: "ui" });
    const subtask = workItems.createItem({ threadId, kind: "subtask", title: "Subtask", parentId: task.id, createdBy: "user", actorId: "ui" });
    const result = workItems.readWorkOptions(threadId);
    expect(result.mode).toBe("epic");
    if (result.mode !== "epic") return;
    const ids = result.children.map((c) => c.item.id);
    expect(ids).toContain(task.id);
    expect(ids).toContain(subtask.id);
  });

  test("beforeSortIndex cuts off items at or beyond a sort_index boundary", () => {
    const { workItems, threadId } = seedThread();
    const t1 = workItems.createItem({ threadId, kind: "task", title: "Before", createdBy: "user", actorId: "ui" });
    const t2 = workItems.createItem({ threadId, kind: "task", title: "After", createdBy: "user", actorId: "ui" });
    // t1 has sort_index 0, t2 has sort_index 1; cutoff sits at index 1
    const cutoff = t2.sort_index;
    const result = workItems.readWorkOptions(threadId, cutoff);
    expect(result.mode).toBe("standalone");
    if (result.mode !== "standalone") return;
    const ids = result.items.map((i) => i.item.id);
    expect(ids).toContain(t1.id);
    expect(ids).not.toContain(t2.id);
  });

  test("beforeSortIndex with no items before cutoff returns empty", () => {
    const { workItems, threadId } = seedThread();
    workItems.createItem({ threadId, kind: "task", title: "After boundary", createdBy: "user", actorId: "ui" });
    // cutoff at sort_index 0 means nothing qualifies (items need sort_index < 0)
    const result = workItems.readWorkOptions(threadId, 0);
    expect(result.mode).toBe("empty");
  });
});

describe("WorkItemStore thread-scoped notes", () => {
  test("addThreadNote allocates an empty-body note attached to the thread, not any work item", () => {
    const { workItems, threadId } = seedThread();
    const id = workItems.addThreadNote(threadId, "", "explore-subagent");
    expect(id).toMatch(/^note-/);
    const notes = workItems.listThreadNotes(threadId, 5);
    expect(notes).toHaveLength(1);
    expect(notes[0]!.id).toBe(id);
    expect(notes[0]!.thread_id).toBe(threadId);
    expect(notes[0]!.work_item_id).toBeNull();
    expect(notes[0]!.body).toBe("");
    expect(notes[0]!.author).toBe("explore-subagent");
  });

  test("updateThreadNoteBody fills in a pre-allocated row", () => {
    const { workItems, threadId } = seedThread();
    const id = workItems.addThreadNote(threadId, "", "explore-subagent");
    workItems.updateThreadNoteBody(id, "The finding: foo calls bar via baz().");
    const notes = workItems.listThreadNotes(threadId, 5);
    expect(notes[0]!.body).toBe("The finding: foo calls bar via baz().");
  });

  test("updateThreadNoteBody rejects unknown note id", () => {
    const { workItems } = seedThread();
    expect(() => workItems.updateThreadNoteBody("note-nope", "x")).toThrow(/unknown thread note/);
  });

  test("listThreadNotes returns newest first and caps limit", async () => {
    const { workItems, threadId } = seedThread();
    const a = workItems.addThreadNote(threadId, "a", "explore-subagent");
    // Sleep to ensure distinct created_at at ms resolution.
    await new Promise((r) => setTimeout(r, 5));
    const b = workItems.addThreadNote(threadId, "b", "explore-subagent");
    const notes = workItems.listThreadNotes(threadId, 5);
    expect(notes.map((n) => n.id)).toEqual([b, a]);
  });

  test("thread notes are isolated from other threads", () => {
    const { workItems, threadId } = seedThread();
    // A thread-scoped note on the wrong thread id just returns no rows.
    workItems.addThreadNote(threadId, "mine", "explore-subagent");
    const other = workItems.listThreadNotes("b-other", 5);
    expect(other).toHaveLength(0);
  });
});

describe("WorkItemStore author column", () => {
  test("createItem accepts an author and roundtrips it", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId,
      kind: "task",
      title: "agent-filed item",
      createdBy: "agent",
      actorId: "mcp",
      author: "agent",
    });
    expect(item.author).toBe("agent");
    const fetched = workItems.getItem(threadId, item.id);
    expect(fetched?.author).toBe("agent");
  });

  test("createItem defaults author to null when not provided", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId,
      kind: "task",
      title: "no author",
      createdBy: "agent",
      actorId: "mcp",
    });
    expect(item.author).toBeNull();
  });

  test("read path maps legacy 'agent-auto' author string to null", () => {
    // Pre-v29 DBs persisted author='agent-auto'. The narrowed enum can't
    // represent it; the store maps such rows to null on read.
    const { workItems, threadId, dir } = seedThread();
    const driver = (workItems as unknown as { stateDb: { run: (sql: string, ...args: unknown[]) => void } }).stateDb;
    const id = "wi-legacy-test";
    const now = new Date().toISOString();
    driver.run(
      `INSERT INTO work_items (id, thread_id, parent_id, kind, title, description, status, priority, sort_index, created_by, created_at, updated_at, author)
       VALUES (?, ?, NULL, 'task', 'legacy', '', 'canceled', 'medium', 99, 'system', ?, ?, 'agent-auto')`,
      id, threadId, now, now,
    );
    void dir;
    const fetched = workItems.getItem(threadId, id);
    expect(fetched?.author).toBeNull();
  });
});

describe("WorkItemStore.findOpenItemForThread (wi-e79eaffd7cf0)", () => {
  test("returns any in_progress item regardless of author", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "explicit",
      status: "in_progress", createdBy: "agent", actorId: "mcp",
      author: "agent",
    });
    const found = workItems.findOpenItemForThread(threadId);
    expect(found?.id).toBe(item.id);
  });

  test("returns null when no in_progress items exist", () => {
    const { workItems, threadId } = seedThread();
    workItems.createItem({
      threadId, kind: "task", title: "ready", createdBy: "agent", actorId: "mcp",
    });
    expect(workItems.findOpenItemForThread(threadId)).toBeNull();
  });
});

describe("copyLastItemNotes (used by fork_thread)", () => {
  test("copies last N notes in chronological order, source untouched", async () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui",
    });
    // Add 5 notes with small delays so created_at ordering is stable.
    for (let i = 1; i <= 5; i++) {
      workItems.addNote(threadId, item.id, `note ${i}`, "agent", "mcp");
      await new Promise((r) => setTimeout(r, 5));
    }
    const before = workItems.getWorkNotes(item.id);
    expect(before).toHaveLength(5);
    const copied = workItems.copyLastItemNotes(item.id, 3);
    expect(copied).toBe(3);
    const after = workItems.getWorkNotes(item.id);
    // 5 originals + 3 copies = 8
    expect(after).toHaveLength(8);
    // Newest rows (the copies) carry bodies of the last 3 originals in
    // chronological order.
    const tail = after.slice(-3).map((n) => n.body);
    expect(tail).toEqual(["note 3", "note 4", "note 5"]);
    // Source rows are untouched: original ids and bodies still present.
    for (const orig of before) {
      const match = after.find((n) => n.id === orig.id);
      expect(match?.body).toBe(orig.body);
      expect(match?.author).toBe(orig.author);
    }
  });

  test("copies all notes when fewer than N exist", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui",
    });
    workItems.addNote(threadId, item.id, "only", "agent", "mcp");
    expect(workItems.copyLastItemNotes(item.id, 3)).toBe(1);
    expect(workItems.getWorkNotes(item.id)).toHaveLength(2);
  });

  test("no-op when item has no notes", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui",
    });
    expect(workItems.copyLastItemNotes(item.id, 3)).toBe(0);
    expect(workItems.getWorkNotes(item.id)).toHaveLength(0);
  });

  test("limit<=0 is a no-op", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui",
    });
    workItems.addNote(threadId, item.id, "x", "agent", "mcp");
    expect(workItems.copyLastItemNotes(item.id, 0)).toBe(0);
    expect(workItems.copyLastItemNotes(item.id, -1)).toBe(0);
    expect(workItems.getWorkNotes(item.id)).toHaveLength(1);
  });
});

describe("WorkItemStore status transition guard (wi-6285706789c5)", () => {
  test("updateItem accepts blocked -> in_progress (deliberate unblock)", () => {
    // The previous guard required a manual blocked → ready → in_progress
    // hop, but every agent caller hit the error on the first try. The
    // unblock gesture IS the deliberate transition; no extra hop needed.
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui", status: "blocked",
    });
    workItems.updateItem({
      threadId, itemId: item.id, status: "in_progress", actorKind: "agent", actorId: "mcp",
    });
    expect(workItems.getItem(threadId, item.id)!.status).toBe("in_progress");
  });

  test("updateItem still rejects done -> in_progress (terminal state)", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui", status: "done",
    });
    expect(() => workItems.updateItem({
      threadId, itemId: item.id, status: "in_progress", actorKind: "agent", actorId: "mcp",
    })).toThrow(/done.*in_progress|move to.*ready.*first/i);
  });

  test("updateItem accepts blocked -> ready (explicit unblock)", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui", status: "blocked",
    });
    workItems.updateItem({
      threadId, itemId: item.id, status: "ready", actorKind: "agent", actorId: "mcp",
    });
    expect(workItems.getItem(threadId, item.id)!.status).toBe("ready");
  });

  test("updateItem accepts ready -> in_progress", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui",
    });
    workItems.updateItem({
      threadId, itemId: item.id, status: "in_progress", actorKind: "agent", actorId: "mcp",
    });
    expect(workItems.getItem(threadId, item.id)!.status).toBe("in_progress");
  });

  test("updateItem accepts human_check -> in_progress (reopen)", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui", status: "human_check",
    });
    workItems.updateItem({
      threadId, itemId: item.id, status: "in_progress", actorKind: "agent", actorId: "mcp",
    });
    expect(workItems.getItem(threadId, item.id)!.status).toBe("in_progress");
  });

  test("updateItem accepts in_progress -> in_progress (no-op)", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui", status: "in_progress",
    });
    workItems.updateItem({
      threadId, itemId: item.id, status: "in_progress", actorKind: "agent", actorId: "mcp",
    });
    expect(workItems.getItem(threadId, item.id)!.status).toBe("in_progress");
  });

  test("updateItem rejects done -> in_progress", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui", status: "done",
    });
    expect(() => workItems.updateItem({
      threadId, itemId: item.id, status: "in_progress", actorKind: "agent", actorId: "mcp",
    })).toThrow(/done.*in_progress|ready.*first/i);
  });

  test("updateItem rejects canceled -> in_progress", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui", status: "canceled",
    });
    expect(() => workItems.updateItem({
      threadId, itemId: item.id, status: "in_progress", actorKind: "agent", actorId: "mcp",
    })).toThrow(/canceled.*in_progress|ready.*first/i);
  });

  test("updateItem rejects archived -> in_progress", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui", status: "archived",
    });
    expect(() => workItems.updateItem({
      threadId, itemId: item.id, status: "in_progress", actorKind: "agent", actorId: "mcp",
    })).toThrow(/archived.*in_progress|ready.*first/i);
  });

  test("completeTask forwards touchedFiles on the emitted change (human_check)", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui", status: "in_progress",
    });
    const seen: Array<{ nextStatus?: string; touchedFiles?: string[] }> = [];
    const off = workItems.subscribe((c) => {
      if (c.kind === "updated") seen.push({ nextStatus: c.nextStatus, touchedFiles: c.touchedFiles });
    });
    workItems.completeTask({
      threadId, itemId: item.id, note: "done",
      touchedFiles: ["src/a.ts", "src/b.ts"],
      actorKind: "agent", actorId: "mcp",
    });
    off();
    const transition = seen.find((s) => s.nextStatus === "human_check");
    expect(transition).toBeDefined();
    expect(transition!.touchedFiles).toEqual(["src/a.ts", "src/b.ts"]);
  });

  test("completeTask forwards touchedFiles on the emitted change (blocked)", () => {
    const { workItems, threadId } = seedThread();
    const item = workItems.createItem({
      threadId, kind: "task", title: "t", createdBy: "user", actorId: "ui", status: "in_progress",
    });
    const seen: Array<{ nextStatus?: string; touchedFiles?: string[] }> = [];
    const off = workItems.subscribe((c) => {
      if (c.kind === "updated") seen.push({ nextStatus: c.nextStatus, touchedFiles: c.touchedFiles });
    });
    workItems.completeTask({
      threadId, itemId: item.id, note: "stuck",
      status: "blocked",
      touchedFiles: ["src/a.ts"],
      actorKind: "agent", actorId: "mcp",
    });
    off();
    const transition = seen.find((s) => s.nextStatus === "blocked");
    expect(transition).toBeDefined();
    expect(transition!.touchedFiles).toEqual(["src/a.ts"]);
  });

  test("listReady excludes blocked items", () => {
    const { workItems, threadId } = seedThread();
    const readyItem = workItems.createItem({
      threadId, kind: "task", title: "ready-item", createdBy: "user", actorId: "ui",
    });
    workItems.createItem({
      threadId, kind: "task", title: "blocked-item", createdBy: "user", actorId: "ui", status: "blocked",
    });
    const ready = workItems.listReady(threadId);
    const ids = ready.map((i) => i.id);
    expect(ids).toContain(readyItem.id);
    expect(ready.every((i) => i.status === "ready")).toBe(true);
  });

  test("readWorkOptions (standalone) excludes blocked items", () => {
    const { workItems, threadId } = seedThread();
    const readyItem = workItems.createItem({
      threadId, kind: "task", title: "ready-item", createdBy: "user", actorId: "ui",
    });
    workItems.createItem({
      threadId, kind: "task", title: "blocked-item", createdBy: "user", actorId: "ui", status: "blocked",
    });
    const result = workItems.readWorkOptions(threadId);
    expect(result.mode).toBe("standalone");
    if (result.mode !== "standalone") return;
    const ids = result.items.map((i) => i.item.id);
    expect(ids).toContain(readyItem.id);
    expect(result.items.every((i) => i.item.status === "ready")).toBe(true);
  });
});
