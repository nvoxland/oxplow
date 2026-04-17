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
