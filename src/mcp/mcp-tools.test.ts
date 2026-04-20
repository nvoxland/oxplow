import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BatchStore } from "../persistence/batch-store.js";
import { WorkItemStore } from "../persistence/work-item-store.js";
import type { Stream } from "../persistence/stream-store.js";
import { buildWorkItemMcpTools, descriptionLooksLikeEmbeddedCriteria, slimWorkItemEvent } from "./mcp-tools.js";
import type { ToolDef } from "./mcp-server.js";

function makeStream(id: string, title: string): Stream {
  return {
    id,
    title,
    summary: "",
    branch: "main",
    branch_ref: "refs/heads/main",
    branch_source: "local",
    worktree_path: `/tmp/${id}`,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    panes: { working: `newde-${id}:working`, talking: `newde-${id}:talking` },
    resume: { working_session_id: "", talking_session_id: "" },
  };
}

function seed() {
  const dir = mkdtempSync(join(tmpdir(), "newde-mcp-tools-"));
  const batchStore = new BatchStore(dir);
  const workItemStore = new WorkItemStore(dir);
  const streamA = makeStream("s-A", "Alpha");
  const streamB = makeStream("s-B", "Beta");
  const stateA = batchStore.ensureStream(streamA);
  const stateB = batchStore.ensureStream(streamB);
  const batchA = stateA.batches[0]!;
  const batchB = stateB.batches[0]!;
  const streams = new Map([
    [streamA.id, streamA],
    [streamB.id, streamB],
  ]);
  const tools = buildWorkItemMcpTools({
    resolveStream: (streamId) => {
      // Pretend streamA is the "current" stream by default.
      if (!streamId) return streamA;
      const found = streams.get(streamId);
      if (!found) throw new Error(`unknown stream: ${streamId}`);
      return found;
    },
    resolveBatchById: (batchId) => {
      const batch = batchStore.findById(batchId);
      if (!batch) throw new Error(`unknown batch: ${batchId}`);
      return batch;
    },
    batchStore,
    streamStore: { list: () => [streamA, streamB] } as never,
    workItemStore,
    commitPointStore: null as never,
    turnStore: null as never,
    fileChangeStore: null as never,
  });
  return { tools, batchStore, workItemStore, streamA, streamB, batchA, batchB };
}

function tool(tools: ToolDef[], name: string): ToolDef {
  const hit = tools.find((t) => t.name === name);
  if (!hit) throw new Error(`tool not found: ${name}`);
  return hit;
}

describe("MCP work-item tools: streamId is inferred from batchId", () => {
  test("list_batch_work with only batchId resolves against the batch's own stream", async () => {
    const { tools, batchB } = seed();
    const t = tool(tools, "newde__list_batch_work");
    // No streamId — old code would throw "unknown batch: b-…" because
    // resolveStream defaults to streamA and streamA doesn't own batchB.
    // New code derives the stream from the batch row.
    const result = await t.handler({ batchId: batchB.id } as never);
    expect((result as { batchId: string }).batchId).toBe(batchB.id);
  });

  test("create_work_item with only batchId lands in the right batch", async () => {
    const { tools, workItemStore, batchB } = seed();
    const t = tool(tools, "newde__create_work_item");
    const result = await t.handler({
      batchId: batchB.id,
      kind: "task",
      title: "streamId-free create",
    } as never);
    const itemId = (result as { id: string }).id;
    const fetched = workItemStore.getItem(batchB.id, itemId);
    expect(fetched?.batch_id).toBe(batchB.id);
    expect(fetched?.title).toBe("streamId-free create");
  });

  test("mismatched streamId is ignored in favour of the batch's real stream", async () => {
    const { tools, streamA, batchB } = seed();
    const t = tool(tools, "newde__list_batch_work");
    // Lying about streamId (claiming streamA when batchB lives in streamB):
    // the old code would pass the wrong streamId to getBatch and throw.
    // The new code trusts the batch row.
    const result = await t.handler({ streamId: streamA.id, batchId: batchB.id } as never);
    expect((result as { batchId: string }).batchId).toBe(batchB.id);
  });

  test("get_batch_context exposes otherActiveBatches across every peer stream", async () => {
    const { tools, streamB, batchB } = seed();
    const t = tool(tools, "newde__get_batch_context");
    // Default-stream call — streamA is "current"; streamB should appear in
    // the cross-stream snapshot.
    const result = await t.handler({} as never);
    const others = (result as {
      otherActiveBatches: Array<{ streamId: string; streamTitle: string; batchId: string; batchTitle: string | null; activeBatchId: string | null }>;
    }).otherActiveBatches;
    expect(others).toHaveLength(1);
    expect(others[0]!.streamId).toBe(streamB.id);
    expect(others[0]!.streamTitle).toBe(streamB.title);
    expect(others[0]!.batchId).toBe(batchB.id);
    expect(others[0]!.activeBatchId).toBe(batchB.id);
  });

  test("create_work_item rejects with a soft-error when acceptance criteria looks embedded in description", async () => {
    const { tools, batchB } = seed();
    const t = tool(tools, "newde__create_work_item");
    const result = await t.handler({
      batchId: batchB.id,
      kind: "task",
      title: "Accidental-embed",
      description: "acceptance criteria:\n- item A\n- item B",
      // No acceptanceCriteria field.
    } as never);
    const errText = (result as { error?: string }).error;
    expect(errText).toBeDefined();
    expect(errText).toContain("acceptanceCriteria is a top-level JSON field");
    // The item was NOT created — the response has no `id` field.
    expect((result as { id?: unknown }).id).toBeUndefined();
  });

  test("create_work_item accepts the happy path: acceptanceCriteria promoted to its own field", async () => {
    const { tools, batchB } = seed();
    const t = tool(tools, "newde__create_work_item");
    const result = await t.handler({
      batchId: batchB.id,
      kind: "task",
      title: "Proper fields",
      description: "background prose about why",
      acceptanceCriteria: "- visible condition 1\n- visible condition 2",
    } as never);
    const createdId = (result as { id: string }).id;
    expect(createdId).toMatch(/^wi-/);
    expect((result as { sort_index: number }).sort_index).toBeGreaterThanOrEqual(0);
  });

  test("descriptionLooksLikeEmbeddedCriteria ignores description mentions without a bullet-looking block", () => {
    // Regression: we don't want to trip on "the existing acceptance criteria
    // said …" in a prose description — the guard only fires when there's
    // an obvious checklist shape too.
    expect(descriptionLooksLikeEmbeddedCriteria("We discussed the acceptance criteria last meeting.")).toBe(false);
    expect(descriptionLooksLikeEmbeddedCriteria("acceptance criteria:\n- item A")).toBe(true);
    expect(descriptionLooksLikeEmbeddedCriteria("")).toBe(false);
    expect(descriptionLooksLikeEmbeddedCriteria(null)).toBe(false);
  });

  test("slimWorkItemEvent reduces updated events to the changed keys only", () => {
    const slim = slimWorkItemEvent({
      event_type: "updated",
      actor_kind: "agent",
      created_at: "2024-01-01T00:00:00Z",
      payload_json: JSON.stringify({
        before: { id: "wi-1", title: "T", status: "ready", description: "same" },
        after: { id: "wi-1", title: "T", status: "in_progress", description: "same" },
      }),
    });
    expect(slim.payload).toEqual({ before: { status: "ready" }, after: { status: "in_progress" } });
  });

  test("slimWorkItemEvent preserves non-updated event payloads as-is", () => {
    const slim = slimWorkItemEvent({
      event_type: "note",
      actor_kind: "agent",
      created_at: "2024-01-01T00:00:00Z",
      payload_json: JSON.stringify({ note: "hi" }),
    });
    expect(slim.payload).toEqual({ note: "hi" });
  });

  test("unknown batchId still throws, with the same error text", async () => {
    const { tools } = seed();
    const t = tool(tools, "newde__list_batch_work");
    await expect(async () => t.handler({ batchId: "b-does-not-exist" } as never))
      .toThrow(/unknown batch/);
  });
});
