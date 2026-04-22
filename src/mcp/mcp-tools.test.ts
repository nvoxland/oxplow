import { describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ThreadStore } from "../persistence/thread-store.js";
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
  const threadStore = new ThreadStore(dir);
  const workItemStore = new WorkItemStore(dir);
  const streamA = makeStream("s-A", "Alpha");
  const streamB = makeStream("s-B", "Beta");
  const stateA = threadStore.ensureStream(streamA);
  const stateB = threadStore.ensureStream(streamB);
  const threadA = stateA.threads[0]!;
  const threadB = stateB.threads[0]!;
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
    resolveThreadById: (threadId) => {
      const thread = threadStore.findById(threadId);
      if (!thread) throw new Error(`unknown thread: ${threadId}`);
      return thread;
    },
    threadStore,
    streamStore: { list: () => [streamA, streamB] } as never,
    workItemStore,
    commitPointStore: null as never,
    turnStore: null as never,
    waitPointStore: null as never,
    executeCommit: (() => { throw new Error("not used"); }) as never,
  });
  return { tools, threadStore, workItemStore, streamA, streamB, threadA, threadB };
}

function tool(tools: ToolDef[], name: string): ToolDef {
  const hit = tools.find((t) => t.name === name);
  if (!hit) throw new Error(`tool not found: ${name}`);
  return hit;
}

describe("MCP work-item tools: streamId is inferred from threadId", () => {
  test("list_thread_work with only threadId resolves against the thread's own stream", async () => {
    const { tools, threadB } = seed();
    const t = tool(tools, "newde__list_thread_work");
    // No streamId — old code would throw "unknown thread: b-…" because
    // resolveStream defaults to streamA and streamA doesn't own threadB.
    // New code derives the stream from the thread row.
    const result = await t.handler({ threadId: threadB.id } as never);
    expect((result as { threadId: string }).threadId).toBe(threadB.id);
  });

  test("create_work_item with only threadId lands in the right thread", async () => {
    const { tools, workItemStore, threadB } = seed();
    const t = tool(tools, "newde__create_work_item");
    const result = await t.handler({
      threadId: threadB.id,
      kind: "task",
      title: "streamId-free create",
    } as never);
    const itemId = (result as { id: string }).id;
    const fetched = workItemStore.getItem(threadB.id, itemId);
    expect(fetched?.thread_id).toBe(threadB.id);
    expect(fetched?.title).toBe("streamId-free create");
  });

  test("mismatched streamId is ignored in favour of the thread's real stream", async () => {
    const { tools, streamA, threadB } = seed();
    const t = tool(tools, "newde__list_thread_work");
    // Lying about streamId (claiming streamA when threadB lives in streamB):
    // the old code would pass the wrong streamId to getThread and throw.
    // The new code trusts the thread row.
    const result = await t.handler({ streamId: streamA.id, threadId: threadB.id } as never);
    expect((result as { threadId: string }).threadId).toBe(threadB.id);
  });

  test("get_thread_context exposes otherActiveThreads across every peer stream", async () => {
    const { tools, streamB, threadB } = seed();
    const t = tool(tools, "newde__get_thread_context");
    // Default-stream call — streamA is "current"; streamB should appear in
    // the cross-stream snapshot.
    const result = await t.handler({} as never);
    const others = (result as {
      otherActiveThreads: Array<{ streamId: string; streamTitle: string; threadId: string; threadTitle: string | null; activeThreadId: string | null }>;
    }).otherActiveThreads;
    expect(others).toHaveLength(1);
    expect(others[0]!.streamId).toBe(streamB.id);
    expect(others[0]!.streamTitle).toBe(streamB.title);
    expect(others[0]!.threadId).toBe(threadB.id);
    expect(others[0]!.activeThreadId).toBe(threadB.id);
  });

  test("create_work_item rejects with a soft-error when acceptance criteria looks embedded in description", async () => {
    const { tools, threadB } = seed();
    const t = tool(tools, "newde__create_work_item");
    const result = await t.handler({
      threadId: threadB.id,
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
    const { tools, threadB } = seed();
    const t = tool(tools, "newde__create_work_item");
    const result = await t.handler({
      threadId: threadB.id,
      kind: "task",
      title: "Proper fields",
      description: "background prose about why",
      acceptanceCriteria: "- visible condition 1\n- visible condition 2",
    } as never);
    const createdId = (result as { id: string }).id;
    expect(createdId).toMatch(/^wi-/);
    expect((result as { sort_index: number }).sort_index).toBeGreaterThanOrEqual(0);
  });

  test("create_work_item includes a reminder field when kind=\"epic\" so the orchestrator files children in the same turn", async () => {
    const { tools, threadB } = seed();
    const t = tool(tools, "newde__create_work_item");
    const result = await t.handler({
      threadId: threadB.id,
      kind: "epic",
      title: "A big rollup",
    } as never) as { id: string; reminder?: string };
    expect(result.id).toMatch(/^wi-/);
    expect(result.reminder).toBeDefined();
    expect(result.reminder).toContain("Epic filed with 0 children");
    expect(result.reminder).toContain("file child tasks now");
    expect(result.reminder).toContain("parentId=this id");
  });

  test("create_work_item does NOT include a reminder for non-epic kinds (happy path stays terse)", async () => {
    const { tools, threadB } = seed();
    const t = tool(tools, "newde__create_work_item");
    for (const kind of ["task", "subtask", "bug", "note"] as const) {
      const result = await t.handler({
        threadId: threadB.id,
        kind,
        title: `A ${kind}`,
      } as never) as { id: string; reminder?: string };
      expect(result.id).toMatch(/^wi-/);
      expect(result.reminder).toBeUndefined();
    }
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

  test("unknown threadId still throws, with the same error text", async () => {
    const { tools } = seed();
    const t = tool(tools, "newde__list_thread_work");
    await expect(async () => t.handler({ threadId: "b-does-not-exist" } as never))
      .toThrow(/unknown thread/);
  });

  test("transition_work_items flips multiple items, fires events per item, same effect as individual calls", async () => {
    const { tools, workItemStore, threadB } = seed();
    const create = tool(tools, "newde__create_work_item");
    const a = (await create.handler({ threadId: threadB.id, kind: "task", title: "A" } as never)) as { id: string };
    const b = (await create.handler({ threadId: threadB.id, kind: "task", title: "B" } as never)) as { id: string };
    const c = (await create.handler({ threadId: threadB.id, kind: "task", title: "C" } as never)) as { id: string };

    const changes: Array<{ itemId: string | null; kind: string }> = [];
    workItemStore.subscribe((change) => changes.push({ itemId: change.itemId, kind: change.kind }));

    const t = tool(tools, "newde__transition_work_items");
    const result = (await t.handler({
      transitions: [
        { threadId: threadB.id, itemId: a.id, status: "in_progress" },
        { threadId: threadB.id, itemId: b.id, status: "in_progress" },
        { threadId: threadB.id, itemId: c.id, status: "human_check" },
      ],
    } as never)) as { ok: boolean; results: Array<{ id: string; status: string }> };
    expect(result.ok).toBe(true);
    expect(result.results).toHaveLength(3);
    expect(result.results.map((r) => r.status)).toEqual(["in_progress", "in_progress", "human_check"]);

    // One `updated` event per transitioned item, same as three individual
    // update_work_item calls would have emitted.
    const updatedFor = new Set(changes.filter((c) => c.kind === "updated").map((c) => c.itemId));
    expect(updatedFor.has(a.id)).toBe(true);
    expect(updatedFor.has(b.id)).toBe(true);
    expect(updatedFor.has(c.id)).toBe(true);

    // Store state reflects the final status of each item.
    expect(workItemStore.getItem(threadB.id, a.id)!.status).toBe("in_progress");
    expect(workItemStore.getItem(threadB.id, b.id)!.status).toBe("in_progress");
    expect(workItemStore.getItem(threadB.id, c.id)!.status).toBe("human_check");
  });

  test("transition_work_items rejects an unknown threadId before firing any side effects", async () => {
    const { tools, workItemStore, threadB } = seed();
    const create = tool(tools, "newde__create_work_item");
    const a = (await create.handler({ threadId: threadB.id, kind: "task", title: "A" } as never)) as { id: string };

    const t = tool(tools, "newde__transition_work_items");
    await expect(async () =>
      t.handler({
        transitions: [
          { threadId: threadB.id, itemId: a.id, status: "in_progress" },
          { threadId: "b-does-not-exist", itemId: "wi-nope", status: "in_progress" },
        ],
      } as never),
    ).toThrow(/unknown thread/);
    // `a` was never flipped — validation happens up front.
    expect(workItemStore.getItem(threadB.id, a.id)!.status).toBe("ready");
  });
});
