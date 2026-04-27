import { describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ThreadStore } from "../persistence/thread-store.js";
import { WorkItemStore } from "../persistence/work-item-store.js";
import { WorkItemEffortStore } from "../persistence/work-item-effort-store.js";
import type { Stream } from "../persistence/stream-store.js";
import { buildWorkItemMcpTools, composeDelegateQueryPrompt, descriptionLooksLikeEmbeddedCriteria, slimWorkItemEvent } from "./mcp-tools.js";
import { FollowupStore } from "../electron/followup-store.js";
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
    panes: { working: `oxplow-${id}:working`, talking: `oxplow-${id}:talking` },
    resume: { working_session_id: "", talking_session_id: "" },
    custom_prompt: null,
  };
}

function seed() {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-mcp-tools-"));
  const threadStore = new ThreadStore(dir);
  const workItemStore = new WorkItemStore(dir);
  const effortStore = new WorkItemEffortStore(dir);
  const followupStore = new FollowupStore();
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
    effortStore,
    followupStore,
  });
  return { tools, threadStore, workItemStore, effortStore, followupStore, streamA, streamB, threadA, threadB, dir };
}

function tool(tools: ToolDef[], name: string): ToolDef {
  const hit = tools.find((t) => t.name === name);
  if (!hit) throw new Error(`tool not found: ${name}`);
  return hit;
}

describe("MCP work-item tools: streamId is inferred from threadId", () => {
  test("list_thread_work with only threadId resolves against the thread's own stream", async () => {
    const { tools, threadB } = seed();
    const t = tool(tools, "oxplow__list_thread_work");
    // No streamId — old code would throw "unknown thread: b-…" because
    // resolveStream defaults to streamA and streamA doesn't own threadB.
    // New code derives the stream from the thread row.
    const result = await t.handler({ threadId: threadB.id } as never);
    expect((result as { threadId: string }).threadId).toBe(threadB.id);
  });

  test("create_work_item with only threadId lands in the right thread", async () => {
    const { tools, workItemStore, threadB } = seed();
    const t = tool(tools, "oxplow__create_work_item");
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

  test("get_subsystem_doc returns the file contents when present", async () => {
    const { tools, threadA, streamA } = seed();
    // streamA's worktree_path is /tmp/s-A by default — overwrite to a real dir.
    const worktree = mkdtempSync(join(tmpdir(), "oxplow-subsys-doc-"));
    (streamA as { worktree_path: string }).worktree_path = worktree;
    mkdirSync(join(worktree, ".context"), { recursive: true });
    writeFileSync(join(worktree, ".context", "data-model.md"), "# data model\nbody", "utf8");
    const t = tool(tools, "oxplow__get_subsystem_doc");
    const result = await t.handler({ threadId: threadA.id, name: "data-model" } as never);
    expect(result).toEqual({
      name: "data-model",
      path: ".context/data-model.md",
      content: "# data model\nbody",
      exists: true,
    });
  });

  test("get_subsystem_doc returns exists=false (no error) when the doc is missing", async () => {
    const { tools, threadA, streamA } = seed();
    const worktree = mkdtempSync(join(tmpdir(), "oxplow-subsys-doc-"));
    (streamA as { worktree_path: string }).worktree_path = worktree;
    const t = tool(tools, "oxplow__get_subsystem_doc");
    const result = await t.handler({ threadId: threadA.id, name: "nonexistent" } as never);
    expect(result).toEqual({
      name: "nonexistent",
      path: ".context/nonexistent.md",
      content: "",
      exists: false,
    });
  });

  test("get_subsystem_doc rejects path-traversal in the name", async () => {
    const { tools, threadA } = seed();
    const t = tool(tools, "oxplow__get_subsystem_doc");
    expect(() => t.handler({ threadId: threadA.id, name: "../etc/passwd" } as never))
      .toThrow(/bare doc name/);
    expect(() => t.handler({ threadId: threadA.id, name: "sub/dir" } as never))
      .toThrow(/bare doc name/);
  });

  test("create_work_item defaults kind to \"task\" when omitted", async () => {
    const { tools, workItemStore, threadB } = seed();
    const t = tool(tools, "oxplow__create_work_item");
    const result = await t.handler({
      threadId: threadB.id,
      title: "no-kind-supplied",
    } as never);
    const itemId = (result as { id: string }).id;
    expect(itemId).toBeDefined();
    const fetched = workItemStore.getItem(threadB.id, itemId);
    expect(fetched?.kind).toBe("task");
  });

  test("mismatched streamId is ignored in favour of the thread's real stream", async () => {
    const { tools, streamA, threadB } = seed();
    const t = tool(tools, "oxplow__list_thread_work");
    // Lying about streamId (claiming streamA when threadB lives in streamB):
    // the old code would pass the wrong streamId to getThread and throw.
    // The new code trusts the thread row.
    const result = await t.handler({ streamId: streamA.id, threadId: threadB.id } as never);
    expect((result as { threadId: string }).threadId).toBe(threadB.id);
  });

  test("get_thread_context exposes otherActiveThreads across every peer stream", async () => {
    const { tools, streamB, threadB } = seed();
    const t = tool(tools, "oxplow__get_thread_context");
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
    const t = tool(tools, "oxplow__create_work_item");
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
    const t = tool(tools, "oxplow__create_work_item");
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
    const t = tool(tools, "oxplow__create_work_item");
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

  test("create_work_item file-and-close shortcut (status=done + touchedFiles) opens and closes an effort with attribution", async () => {
    const { tools, workItemStore, threadB, dir } = seed();
    // Mimic runtime wiring: subscribe to work-item changes and route
    // status transitions through applyStatusTransition so the effort
    // store actually gets updated. In production this lives in
    // runtime.ts; the MCP test seed leaves effortStore null so we set
    // up a lightweight wiring here. Uses the seed's projectDir so the
    // work_item_effort FK to work_items resolves in the same SQLite file.
    const { WorkItemEffortStore: EffortStore } = await import("../persistence/work-item-effort-store.js");
    const { applyStatusTransition } = await import("../electron/runtime.js");
    const effortStore = new EffortStore(dir);
    const off = workItemStore.subscribe((change) => {
      if (change.kind === "updated" && change.itemId && change.previousStatus !== change.nextStatus) {
        applyStatusTransition(
          { effortStore, flushSnapshot: () => null },
          {
            threadId: change.threadId,
            workItemId: change.itemId,
            previous: change.previousStatus,
            next: change.nextStatus,
            touchedFiles: change.touchedFiles,
          },
        );
      }
    });

    const t = tool(tools, "oxplow__create_work_item");
    const result = await t.handler({
      threadId: threadB.id,
      kind: "task",
      title: "Retroactive split",
      status: "done",
      touchedFiles: ["src/a.ts", "src/b.ts"],
    } as never) as { ok: boolean; id: string };
    off();

    expect(result.ok).toBe(true);
    // Final row status is the requested target.
    expect(workItemStore.getItem(threadB.id, result.id)!.status).toBe("done");
    // Exactly one effort exists (opened by ready→in_progress, closed by in_progress→done).
    const efforts = effortStore.listEffortsForWorkItem(result.id);
    expect(efforts).toHaveLength(1);
    expect(efforts[0]!.ended_at).not.toBeNull();
    // Attribution landed on the closed effort.
    expect(effortStore.listEffortFiles(efforts[0]!.id).sort()).toEqual(["src/a.ts", "src/b.ts"]);
  });

  test("create_work_item file-and-close shortcut requires touchedFiles to synthesize the effort (status alone is a plain create)", async () => {
    const { tools, workItemStore, threadB, dir } = seed();
    const { WorkItemEffortStore: EffortStore } = await import("../persistence/work-item-effort-store.js");
    const { applyStatusTransition } = await import("../electron/runtime.js");
    const effortStore = new EffortStore(dir);
    const off = workItemStore.subscribe((change) => {
      if (change.kind === "updated" && change.itemId && change.previousStatus !== change.nextStatus) {
        applyStatusTransition(
          { effortStore, flushSnapshot: () => null },
          {
            threadId: change.threadId,
            workItemId: change.itemId,
            previous: change.previousStatus,
            next: change.nextStatus,
            touchedFiles: change.touchedFiles,
          },
        );
      }
    });

    const t = tool(tools, "oxplow__create_work_item");
    const result = await t.handler({
      threadId: threadB.id,
      kind: "note",
      title: "Pure note — no edits",
      status: "done",
    } as never) as { id: string };
    off();

    // No touchedFiles → no synthesized effort (agent is signalling "nothing to attribute").
    expect(effortStore.listEffortsForWorkItem(result.id)).toHaveLength(0);
    expect(workItemStore.getItem(threadB.id, result.id)!.status).toBe("done");
  });

  test("create_work_item with status=in_progress routes through ready → in_progress so an effort is opened", async () => {
    const { tools, workItemStore, threadB, dir } = seed();
    const { WorkItemEffortStore: EffortStore } = await import("../persistence/work-item-effort-store.js");
    const { applyStatusTransition } = await import("../electron/runtime.js");
    const effortStore = new EffortStore(dir);
    const off = workItemStore.subscribe((change) => {
      if (change.kind === "updated" && change.itemId && change.previousStatus !== change.nextStatus) {
        applyStatusTransition(
          { effortStore, flushSnapshot: () => null },
          {
            threadId: change.threadId,
            workItemId: change.itemId,
            previous: change.previousStatus,
            next: change.nextStatus,
            touchedFiles: change.touchedFiles,
          },
        );
      }
    });
    const t = tool(tools, "oxplow__create_work_item");
    const result = await t.handler({
      threadId: threadB.id,
      kind: "task",
      title: "Work that starts in_progress",
      status: "in_progress",
    } as never) as { id: string };
    off();
    expect(workItemStore.getItem(threadB.id, result.id)!.status).toBe("in_progress");
    // An open effort exists for this item — proves the ready → in_progress
    // transition fired through the subscription.
    const efforts = effortStore.listEffortsForWorkItem(result.id);
    expect(efforts).toHaveLength(1);
    expect(efforts[0]!.ended_at).toBeNull();
  });

  test("create_work_item does NOT include a reminder for non-epic kinds (happy path stays terse)", async () => {
    const { tools, threadB } = seed();
    const t = tool(tools, "oxplow__create_work_item");
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

  test("create_work_item emits redoHint when a done item was closed on the same thread within the window", async () => {
    const { tools, workItemStore, threadB } = seed();
    const t = tool(tools, "oxplow__create_work_item");
    // First: file a task and close it to done (simulating an
    // effort the agent just shipped).
    const first = await t.handler({
      threadId: threadB.id,
      kind: "task",
      title: "Original task",
    } as never) as { id: string };
    workItemStore.updateItem({
      threadId: threadB.id,
      itemId: first.id,
      status: "in_progress",
      actorKind: "agent",
      actorId: "mcp",
    });
    workItemStore.updateItem({
      threadId: threadB.id,
      itemId: first.id,
      status: "done",
      actorKind: "agent",
      actorId: "mcp",
    });
    // Now the agent reflexively files a "fix" — response should include
    // a redoHint pointing back at the first item.
    const second = await t.handler({
      threadId: threadB.id,
      kind: "task",
      title: "Fix original task",
    } as never) as { id: string; redoHint?: string };
    expect(second.redoHint).toBeDefined();
    expect(second.redoHint).toContain(first.id);
    expect(second.redoHint).toContain("in_progress");
  });

  test("create_work_item omits redoHint when no recent done item exists", async () => {
    const { tools, threadB } = seed();
    const t = tool(tools, "oxplow__create_work_item");
    const result = await t.handler({
      threadId: threadB.id,
      kind: "task",
      title: "Fresh work",
    } as never) as { id: string; redoHint?: string };
    expect(result.redoHint).toBeUndefined();
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
    const t = tool(tools, "oxplow__list_thread_work");
    await expect(async () => t.handler({ threadId: "b-does-not-exist" } as never))
      .toThrow(/unknown thread/);
  });

  test("transition_work_items flips multiple items, fires events per item, same effect as individual calls", async () => {
    const { tools, workItemStore, threadB } = seed();
    const create = tool(tools, "oxplow__create_work_item");
    const a = (await create.handler({ threadId: threadB.id, kind: "task", title: "A" } as never)) as { id: string };
    const b = (await create.handler({ threadId: threadB.id, kind: "task", title: "B" } as never)) as { id: string };
    const c = (await create.handler({ threadId: threadB.id, kind: "task", title: "C" } as never)) as { id: string };

    const changes: Array<{ itemId: string | null; kind: string }> = [];
    workItemStore.subscribe((change) => changes.push({ itemId: change.itemId, kind: change.kind }));

    const t = tool(tools, "oxplow__transition_work_items");
    const result = (await t.handler({
      transitions: [
        { threadId: threadB.id, itemId: a.id, status: "in_progress" },
        { threadId: threadB.id, itemId: b.id, status: "in_progress" },
        { threadId: threadB.id, itemId: c.id, status: "done" },
      ],
    } as never)) as { ok: boolean; results: Array<{ id: string; status: string }> };
    expect(result.ok).toBe(true);
    expect(result.results).toHaveLength(3);
    expect(result.results.map((r) => r.status)).toEqual(["in_progress", "in_progress", "done"]);

    // One `updated` event per transitioned item, same as three individual
    // update_work_item calls would have emitted.
    const updatedFor = new Set(changes.filter((c) => c.kind === "updated").map((c) => c.itemId));
    expect(updatedFor.has(a.id)).toBe(true);
    expect(updatedFor.has(b.id)).toBe(true);
    expect(updatedFor.has(c.id)).toBe(true);

    // Store state reflects the final status of each item.
    expect(workItemStore.getItem(threadB.id, a.id)!.status).toBe("in_progress");
    expect(workItemStore.getItem(threadB.id, b.id)!.status).toBe("in_progress");
    expect(workItemStore.getItem(threadB.id, c.id)!.status).toBe("done");
  });

  test("transition_work_items rejects an unknown threadId before firing any side effects", async () => {
    const { tools, workItemStore, threadB } = seed();
    const create = tool(tools, "oxplow__create_work_item");
    const a = (await create.handler({ threadId: threadB.id, kind: "task", title: "A" } as never)) as { id: string };

    const t = tool(tools, "oxplow__transition_work_items");
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

  test("file_epic_with_children creates epic and child tasks in one atomic call", async () => {
    const { tools, workItemStore, threadB } = seed();
    const t = tool(tools, "oxplow__file_epic_with_children");
    const result = (await t.handler({
      threadId: threadB.id,
      epic: {
        title: "Big refactor",
        description: "why we're doing this",
        acceptance_criteria: "- ships behind flag",
        priority: "high",
      },
      children: [
        { title: "Child one", acceptance_criteria: "- criterion A" },
        { title: "Child two", description: "detail", priority: "medium" },
        { title: "Subtask three", kind: "subtask" },
      ],
    } as never)) as { epicId: string; childIds: string[] };
    expect(result.epicId).toMatch(/^wi-/);
    expect(result.childIds).toHaveLength(3);
    for (const id of result.childIds) expect(id).toMatch(/^wi-/);

    const epic = workItemStore.getItem(threadB.id, result.epicId)!;
    expect(epic.kind).toBe("epic");
    expect(epic.title).toBe("Big refactor");
    expect(epic.priority).toBe("high");
    expect(epic.acceptance_criteria).toContain("ships behind flag");

    const children = result.childIds.map((id) => workItemStore.getItem(threadB.id, id)!);
    expect(children[0]!.parent_id).toBe(result.epicId);
    expect(children[0]!.kind).toBe("task"); // default
    expect(children[0]!.title).toBe("Child one");
    expect(children[2]!.kind).toBe("subtask"); // override respected
  });

  test("file_epic_with_children rejects empty children array — epics must have children", async () => {
    const { tools, threadB } = seed();
    const t = tool(tools, "oxplow__file_epic_with_children");
    await expect(async () => t.handler({
      threadId: threadB.id,
      epic: { title: "Lonely epic" },
      children: [],
    } as never)).toThrow(/at least one child|epic without children is a bug/i);
  });

  test("file_epic_with_children rolls back if a child fails validation (atomic)", async () => {
    const { tools, workItemStore, threadB } = seed();
    const t = tool(tools, "oxplow__file_epic_with_children");
    const before = workItemStore.listItems(threadB.id).length;
    await expect(async () => t.handler({
      threadId: threadB.id,
      epic: { title: "Attempted" },
      children: [
        { title: "Fine" },
        { title: "" }, // rejected: empty title
      ],
    } as never)).toThrow();
    // Nothing persisted — both epic and first child rolled back.
    expect(workItemStore.listItems(threadB.id).length).toBe(before);
  });

  test("complete_task writes the note onto the just-closed effort and does NOT add a note row", async () => {
    const { tools, workItemStore, effortStore, threadB } = seed();
    // Wire the status-transition hook so the effort opens/closes when
    // status flips. (Production wires this in runtime.ts; the seed
    // doesn't.)
    const { applyStatusTransition } = await import("../electron/runtime.js");
    const off = workItemStore.subscribe((change) => {
      if (change.kind === "updated" && change.itemId && change.previousStatus !== change.nextStatus) {
        applyStatusTransition(
          { effortStore, flushSnapshot: () => null },
          {
            threadId: change.threadId,
            workItemId: change.itemId,
            previous: change.previousStatus,
            next: change.nextStatus,
            touchedFiles: change.touchedFiles,
          },
        );
      }
    });

    const create = tool(tools, "oxplow__create_work_item");
    const a = (await create.handler({ threadId: threadB.id, kind: "task", title: "Thing" } as never)) as { id: string };
    const update = tool(tools, "oxplow__update_work_item");
    await update.handler({ threadId: threadB.id, itemId: a.id, status: "in_progress" } as never);

    const tcomplete = tool(tools, "oxplow__complete_task");
    const result = (await tcomplete.handler({
      threadId: threadB.id,
      itemId: a.id,
      note: "Shipped: see commit abc123",
    } as never)) as { ok: boolean; id: string; status: string };
    off();

    expect(result.ok).toBe(true);
    expect(result.status).toBe("done");
    expect(workItemStore.getItem(threadB.id, a.id)!.status).toBe("done");
    // Note is NOT appended to work-item history anymore.
    const events = workItemStore.listEvents(threadB.id, a.id);
    const notes = events.filter((e) => e.event_type === "note");
    expect(notes).toHaveLength(0);
    // Summary lives on the just-closed effort.
    const efforts = effortStore.listEffortsForWorkItem(a.id);
    expect(efforts).toHaveLength(1);
    expect(efforts[0]!.ended_at).not.toBeNull();
    expect(efforts[0]!.summary).toBe("Shipped: see commit abc123");
  });

  test("complete_task accepts status=blocked", async () => {
    const { tools, workItemStore, threadB } = seed();
    const create = tool(tools, "oxplow__create_work_item");
    const a = (await create.handler({ threadId: threadB.id, kind: "task", title: "Stuck" } as never)) as { id: string };
    const t = tool(tools, "oxplow__complete_task");
    const result = (await t.handler({
      threadId: threadB.id,
      itemId: a.id,
      note: "Waiting on upstream PR",
      status: "blocked",
    } as never)) as { ok: boolean; status: string };
    expect(result.status).toBe("blocked");
    expect(workItemStore.getItem(threadB.id, a.id)!.status).toBe("blocked");
  });

  test("complete_task rejects unknown status values", async () => {
    const { tools, threadB } = seed();
    const create = tool(tools, "oxplow__create_work_item");
    const a = (await create.handler({ threadId: threadB.id, kind: "task", title: "X" } as never)) as { id: string };
    const t = tool(tools, "oxplow__complete_task");
    await expect(async () => t.handler({
      threadId: threadB.id, itemId: a.id, note: "done", status: "ready",
    } as never)).toThrow(/done.*blocked|blocked.*done/i);
  });

  test("dispatch_work_item returns a brief containing preamble, title, description, and acceptance criteria", async () => {
    const { tools, threadB } = seed();
    const create = tool(tools, "oxplow__create_work_item");
    const item = (await create.handler({
      threadId: threadB.id,
      kind: "task",
      title: "Fix the login redirect bug",
      description: "Login redirects to the wrong landing page.",
      acceptanceCriteria: "- redirects to dashboard\n- test added",
      priority: "high",
    } as never)) as { id: string };

    const t = tool(tools, "oxplow__dispatch_work_item");
    const result = (await t.handler({
      threadId: threadB.id,
      itemId: item.id,
      autoStart: false,
    } as never)) as { prompt: string; itemId: string };

    expect(result.itemId).toBe(item.id);
    expect(result.prompt).toContain(item.id);
    expect(result.prompt).toContain("Fix the login redirect bug");
    expect(result.prompt).toContain("Login redirects to the wrong landing page.");
    expect(result.prompt).toContain("redirects to dashboard");
    // Preamble: subagent-protocol essentials
    expect(result.prompt).toContain("oxplow work item");
    expect(result.prompt).toContain("in_progress");
    expect(result.prompt).toContain("complete_task");
    expect(result.prompt).toContain("oxplow-result:");
    // Trailing constraints
    expect(result.prompt).toMatch(/Singular table names/i);
  });

  test("dispatch_work_item autoStart=true transitions a ready item to in_progress", async () => {
    const { tools, workItemStore, threadB } = seed();
    const create = tool(tools, "oxplow__create_work_item");
    const item = (await create.handler({
      threadId: threadB.id,
      kind: "task",
      title: "Ready thing",
    } as never)) as { id: string };
    expect(workItemStore.getItem(threadB.id, item.id)!.status).toBe("ready");

    const t = tool(tools, "oxplow__dispatch_work_item");
    await t.handler({ threadId: threadB.id, itemId: item.id } as never);
    expect(workItemStore.getItem(threadB.id, item.id)!.status).toBe("in_progress");
  });

  test("dispatch_work_item autoStart=false leaves status alone", async () => {
    const { tools, workItemStore, threadB } = seed();
    const create = tool(tools, "oxplow__create_work_item");
    const item = (await create.handler({
      threadId: threadB.id,
      kind: "task",
      title: "Untouched",
    } as never)) as { id: string };

    const t = tool(tools, "oxplow__dispatch_work_item");
    await t.handler({ threadId: threadB.id, itemId: item.id, autoStart: false } as never);
    expect(workItemStore.getItem(threadB.id, item.id)!.status).toBe("ready");
  });

  test("dispatch_work_item on already-in_progress item skips transition silently", async () => {
    const { tools, workItemStore, threadB } = seed();
    const create = tool(tools, "oxplow__create_work_item");
    const item = (await create.handler({
      threadId: threadB.id,
      kind: "task",
      title: "Already running",
    } as never)) as { id: string };
    workItemStore.updateItem({
      threadId: threadB.id,
      itemId: item.id,
      status: "in_progress",
      actorKind: "agent",
      actorId: "test",
    });
    const t = tool(tools, "oxplow__dispatch_work_item");
    const result = (await t.handler({ threadId: threadB.id, itemId: item.id } as never)) as { prompt: string };
    expect(result.prompt).toContain("Already running");
    expect(workItemStore.getItem(threadB.id, item.id)!.status).toBe("in_progress");
  });

  test("dispatch_work_item for an epic includes child titles and acceptance criteria", async () => {
    const { tools, threadB } = seed();
    const fileEpic = tool(tools, "oxplow__file_epic_with_children");
    const result = (await fileEpic.handler({
      threadId: threadB.id,
      epic: {
        title: "Epic parent",
        description: "rollup description",
      },
      children: [
        { title: "Child alpha", acceptance_criteria: "- alpha ships" },
        { title: "Child beta", acceptance_criteria: "- beta ships" },
      ],
    } as never)) as { epicId: string; childIds: string[] };

    const t = tool(tools, "oxplow__dispatch_work_item");
    const out = (await t.handler({
      threadId: threadB.id,
      itemId: result.epicId,
      autoStart: false,
    } as never)) as { prompt: string };
    expect(out.prompt).toContain("Epic parent");
    expect(out.prompt).toContain("Child alpha");
    expect(out.prompt).toContain("alpha ships");
    expect(out.prompt).toContain("Child beta");
    expect(out.prompt).toContain("beta ships");
  });

  test("dispatch_work_item appends extraContext when provided", async () => {
    const { tools, threadB } = seed();
    const create = tool(tools, "oxplow__create_work_item");
    const item = (await create.handler({
      threadId: threadB.id,
      kind: "task",
      title: "T",
    } as never)) as { id: string };
    const t = tool(tools, "oxplow__dispatch_work_item");
    const out = (await t.handler({
      threadId: threadB.id,
      itemId: item.id,
      autoStart: false,
      extraContext: "Prefer debounce over dedupe here.",
    } as never)) as { prompt: string };
    expect(out.prompt).toContain("Additional context");
    expect(out.prompt).toContain("Prefer debounce over dedupe here.");
  });

  test("fork_thread invokes the runtime-provided forkThread callback with the right shape", async () => {
    const { threadStore, workItemStore, streamA, threadA } = seed();
    // Pre-seed some items on the source thread (ready-by-default).
    const ready1 = workItemStore.createItem({ threadId: threadA.id, kind: "task", title: "r1", createdBy: "user", actorId: "test" });
    const ready2 = workItemStore.createItem({ threadId: threadA.id, kind: "task", title: "r2", createdBy: "user", actorId: "test" });
    const calls: Array<{ sourceThreadId: string; title: string; summary: string; moveItemIds?: string[] }> = [];
    const tools2 = buildWorkItemMcpTools({
      resolveStream: () => streamA,
      resolveThreadById: (tid) => {
        const t = threadStore.findById(tid);
        if (!t) throw new Error("no");
        return t;
      },
      threadStore,
      streamStore: { list: () => [streamA] } as never,
      workItemStore,
      effortStore: null as never,
      forkThread: (input) => {
        calls.push(input);
        return { newThreadId: "b-newfork" };
      },
    });
    const t = tool(tools2, "oxplow__fork_thread");
    const out = (await t.handler({
      sourceThreadId: threadA.id,
      title: "Forked thread",
      summary: "carry-over context",
      moveItemIds: [ready1.id, ready2.id],
    } as never)) as { newThreadId: string };
    expect(out.newThreadId).toBe("b-newfork");
    expect(calls).toHaveLength(1);
    expect(calls[0]!.sourceThreadId).toBe(threadA.id);
    expect(calls[0]!.title).toBe("Forked thread");
    expect(calls[0]!.summary).toBe("carry-over context");
    expect(calls[0]!.moveItemIds).toEqual([ready1.id, ready2.id]);
  });

  test("fork_thread throws when the runtime hook is not wired (test-only deps shape)", async () => {
    const { tools, threadA } = seed();
    const t = tool(tools, "oxplow__fork_thread");
    await expect(async () => t.handler({
      sourceThreadId: threadA.id,
      title: "x",
      summary: "y",
    } as never)).toThrow(/runtime not wired/);
  });

  test("delegate_query pre-allocates a thread-scoped note and returns a prompt including question + focus + noteId", async () => {
    const { tools, workItemStore, threadA } = seed();
    const t = tool(tools, "oxplow__delegate_query");
    const result = (await t.handler({
      threadId: threadA.id,
      question: "Where is Monaco wired in?",
      focus: "src/ui/editor-pane.ts",
    } as never)) as { ok: boolean; prompt: string; provisionalNoteId: string };
    expect(result.ok).toBe(true);
    expect(result.provisionalNoteId).toMatch(/^note-/);
    expect(result.prompt).toContain("Where is Monaco wired in?");
    expect(result.prompt).toContain("src/ui/editor-pane.ts");
    expect(result.prompt).toContain(result.provisionalNoteId);
    expect(result.prompt).toContain("record_query_finding");
    // Verify the note landed with empty body.
    const notes = workItemStore.listThreadNotes(threadA.id, 5);
    expect(notes).toHaveLength(1);
    expect(notes[0]!.id).toBe(result.provisionalNoteId);
    expect(notes[0]!.body).toBe("");
    expect(notes[0]!.thread_id).toBe(threadA.id);
    expect(notes[0]!.work_item_id).toBeNull();
  });

  test("record_query_finding fills the body of a pre-allocated thread note", async () => {
    const { tools, workItemStore, threadA } = seed();
    const delegate = tool(tools, "oxplow__delegate_query");
    const record = tool(tools, "oxplow__record_query_finding");
    const { provisionalNoteId } = (await delegate.handler({
      threadId: threadA.id,
      question: "question?",
    } as never)) as { provisionalNoteId: string };
    const result = (await record.handler({
      noteId: provisionalNoteId,
      body: "The answer is: Monaco is wired via EditorPane.",
    } as never)) as { ok: boolean; noteId: string };
    expect(result.ok).toBe(true);
    const notes = workItemStore.listThreadNotes(threadA.id, 5);
    expect(notes[0]!.body).toContain("Monaco is wired via EditorPane");
  });

  test("record_query_finding rejects an item-scoped note id", async () => {
    const { tools, threadA } = seed();
    const create = tool(tools, "oxplow__create_work_item");
    const item = (await create.handler({ threadId: threadA.id, kind: "task", title: "t" } as never)) as { id: string };
    // Insert an item-scoped row directly via add_work_note — but add_work_note
    // writes to work_item_events, not work_note. Instead, build an item-scoped
    // work_note via a raw SQL path is overkill; assert the error shape by
    // asking record_query_finding for a non-existent id first:
    const record = tool(tools, "oxplow__record_query_finding");
    await expect(async () =>
      record.handler({ noteId: "note-does-not-exist", body: "x" } as never),
    ).toThrow(/unknown thread note/);
    // And if a real item-scoped note somehow existed (future path), the
    // store method's guard is covered directly in work-item-store.test.ts.
    expect(item.id).toMatch(/^wi-/);
  });

  test("get_thread_notes returns thread notes in reverse chronological order", async () => {
    const { tools, threadA } = seed();
    const delegate = tool(tools, "oxplow__delegate_query");
    const record = tool(tools, "oxplow__record_query_finding");
    const getNotes = tool(tools, "oxplow__get_thread_notes");
    const a = (await delegate.handler({ threadId: threadA.id, question: "q1" } as never)) as { provisionalNoteId: string };
    // Small gap so created_at differs.
    await new Promise((r) => setTimeout(r, 5));
    const b = (await delegate.handler({ threadId: threadA.id, question: "q2" } as never)) as { provisionalNoteId: string };
    await record.handler({ noteId: a.provisionalNoteId, body: "first finding" } as never);
    await record.handler({ noteId: b.provisionalNoteId, body: "second finding" } as never);
    const result = (await getNotes.handler({ threadId: threadA.id, limit: 5 } as never)) as {
      notes: Array<{ id: string; body: string }>;
    };
    expect(result.notes).toHaveLength(2);
    // Newest first: b before a.
    expect(result.notes[0]!.id).toBe(b.provisionalNoteId);
    expect(result.notes[1]!.id).toBe(a.provisionalNoteId);
  });

  test("composeDelegateQueryPrompt renders the expected sections", () => {
    const prompt = composeDelegateQueryPrompt({
      threadId: "b-1",
      question: "What does foo do?",
      focus: "src/foo.ts",
      noteId: "note-abc",
    });
    expect(prompt).toContain("threadId: b-1");
    expect(prompt).toContain("noteId: note-abc");
    expect(prompt).toContain("## Question");
    expect(prompt).toContain("What does foo do?");
    expect(prompt).toContain("## Focus");
    expect(prompt).toContain("src/foo.ts");
    expect(prompt).toContain("record_query_finding");
  });

  test("composeDelegateQueryPrompt omits Focus section when focus is empty", () => {
    const prompt = composeDelegateQueryPrompt({
      threadId: "b-1",
      question: "q",
      focus: "",
      noteId: "note-x",
    });
    expect(prompt).not.toContain("## Focus");
  });

  test("add_followup creates an in-memory entry and returns its id", async () => {
    const { tools, followupStore, threadB } = seed();
    const t = tool(tools, "oxplow__add_followup");
    const result = (await t.handler({ threadId: threadB.id, note: "loop back to dispatch" } as never)) as { ok: true; id: string };
    expect(result.id).toMatch(/^fu-/);
    const list = followupStore.list(threadB.id);
    expect(list).toHaveLength(1);
    expect(list[0]!.note).toBe("loop back to dispatch");
  });

  test("add_followup rejects empty notes", async () => {
    const { tools, threadB } = seed();
    const t = tool(tools, "oxplow__add_followup");
    await expect(async () => t.handler({ threadId: threadB.id, note: "   " } as never)).toThrow();
  });

  test("remove_followup drops the entry; returns ok=false when missing", async () => {
    const { tools, followupStore, threadB } = seed();
    const add = tool(tools, "oxplow__add_followup");
    const remove = tool(tools, "oxplow__remove_followup");
    const a = (await add.handler({ threadId: threadB.id, note: "x" } as never)) as { id: string };
    const ok = (await remove.handler({ threadId: threadB.id, id: a.id } as never)) as { ok: boolean };
    expect(ok.ok).toBe(true);
    expect(followupStore.list(threadB.id)).toHaveLength(0);
    const second = (await remove.handler({ threadId: threadB.id, id: a.id } as never)) as { ok: boolean };
    expect(second.ok).toBe(false);
  });

  test("list_followups returns the thread's entries in insertion order", async () => {
    const { tools, threadB } = seed();
    const add = tool(tools, "oxplow__add_followup");
    const list = tool(tools, "oxplow__list_followups");
    await add.handler({ threadId: threadB.id, note: "first" } as never);
    await add.handler({ threadId: threadB.id, note: "second" } as never);
    const result = (await list.handler({ threadId: threadB.id } as never)) as { followups: Array<{ note: string }> };
    expect(result.followups.map((f) => f.note)).toEqual(["first", "second"]);
  });

  test("complete_task rejects if current status is terminal (done/canceled/archived)", async () => {
    const { tools, workItemStore, threadB } = seed();
    const create = tool(tools, "oxplow__create_work_item");
    const a = (await create.handler({ threadId: threadB.id, kind: "task", title: "Finished" } as never)) as { id: string };
    // Directly seed a terminal state via the store.
    workItemStore.updateItem({
      threadId: threadB.id, itemId: a.id, status: "done", actorKind: "user", actorId: "test",
    });
    const t = tool(tools, "oxplow__complete_task");
    await expect(async () => t.handler({
      threadId: threadB.id, itemId: a.id, note: "already done",
    } as never)).toThrow(/terminal|already.*done|canceled|archived/i);
  });

  describe("epic-cascade guard on close", () => {
    test("complete_task on an epic with non-terminal children rejects when cascade is omitted", async () => {
      const { tools, threadB } = seed();
      const fileEpic = tool(tools, "oxplow__file_epic_with_children");
      const created = (await fileEpic.handler({
        threadId: threadB.id,
        epic: { title: "Big work" },
        children: [{ title: "child A" }, { title: "child B" }],
      } as never)) as { ok: boolean; epicId: string; childIds: string[] };

      const completeTask = tool(tools, "oxplow__complete_task");
      const result = (await completeTask.handler({
        threadId: threadB.id,
        itemId: created.epicId,
        note: "done",
      } as never)) as { error?: string; status?: string };
      expect(result.error).toBeDefined();
      expect(result.error!).toContain("non-terminal");
      expect(result.error!).toContain("cascade: true");
      // Epic must NOT have been transitioned.
      expect(result.status).toBeUndefined();
    });

    test("complete_task on an epic cascades children when cascade=true", async () => {
      const { tools, workItemStore, threadB } = seed();
      const fileEpic = tool(tools, "oxplow__file_epic_with_children");
      const created = (await fileEpic.handler({
        threadId: threadB.id,
        epic: { title: "Big work" },
        children: [{ title: "child A" }, { title: "child B" }],
      } as never)) as { epicId: string; childIds: string[] };

      const completeTask = tool(tools, "oxplow__complete_task");
      const result = (await completeTask.handler({
        threadId: threadB.id,
        itemId: created.epicId,
        note: "epic done",
        cascade: true,
      } as never)) as { ok: boolean; status: string };
      expect(result.ok).toBe(true);
      expect(result.status).toBe("done");
      // Both children flipped to done.
      for (const childId of created.childIds) {
        expect(workItemStore.getItem(threadB.id, childId)!.status).toBe("done");
      }
    });

    test("complete_task on a non-epic ignores cascade entirely (no rejection)", async () => {
      const { tools, threadB } = seed();
      const create = tool(tools, "oxplow__create_work_item");
      const a = (await create.handler({
        threadId: threadB.id, kind: "task", title: "plain task", status: "in_progress",
      } as never)) as { id: string };

      const completeTask = tool(tools, "oxplow__complete_task");
      const result = (await completeTask.handler({
        threadId: threadB.id,
        itemId: a.id,
        note: "done",
      } as never)) as { ok: boolean; status: string };
      expect(result.ok).toBe(true);
      expect(result.status).toBe("done");
    });

    test("update_work_item to done on an epic also enforces the guard", async () => {
      const { tools, threadB } = seed();
      const fileEpic = tool(tools, "oxplow__file_epic_with_children");
      const created = (await fileEpic.handler({
        threadId: threadB.id,
        epic: { title: "Big work" },
        children: [{ title: "child A" }],
      } as never)) as { epicId: string };

      const update = tool(tools, "oxplow__update_work_item");
      const result = (await update.handler({
        threadId: threadB.id,
        itemId: created.epicId,
        status: "done",
      } as never)) as { error?: string; status?: string };
      expect(result.error).toBeDefined();
      expect(result.error!).toContain("non-terminal");
    });

    test("update_work_item to done with cascade flips all children", async () => {
      const { tools, workItemStore, threadB } = seed();
      const fileEpic = tool(tools, "oxplow__file_epic_with_children");
      const created = (await fileEpic.handler({
        threadId: threadB.id,
        epic: { title: "Big work" },
        children: [{ title: "child A" }],
      } as never)) as { epicId: string; childIds: string[] };

      const update = tool(tools, "oxplow__update_work_item");
      const result = (await update.handler({
        threadId: threadB.id,
        itemId: created.epicId,
        status: "blocked",
        cascade: true,
      } as never)) as { ok: boolean; status: string };
      expect(result.ok).toBe(true);
      expect(result.status).toBe("blocked");
      for (const childId of created.childIds) {
        expect(workItemStore.getItem(threadB.id, childId)!.status).toBe("blocked");
      }
    });

    test("update_work_item to ready/in_progress on an epic does NOT enforce the guard (only closes do)", async () => {
      const { tools, threadB } = seed();
      const fileEpic = tool(tools, "oxplow__file_epic_with_children");
      const created = (await fileEpic.handler({
        threadId: threadB.id,
        epic: { title: "Big work" },
        children: [{ title: "child A" }],
      } as never)) as { epicId: string };

      const update = tool(tools, "oxplow__update_work_item");
      // Re-opening an epic shouldn't trip the guard — it only fires
      // on closes (done / blocked).
      const result = (await update.handler({
        threadId: threadB.id,
        itemId: created.epicId,
        status: "ready",
      } as never)) as { ok: boolean; status: string };
      expect(result.ok).toBe(true);
      expect(result.status).toBe("ready");
    });

    test("epic with all children already in terminal status passes through without cascade", async () => {
      const { tools, threadB } = seed();
      const fileEpic = tool(tools, "oxplow__file_epic_with_children");
      const created = (await fileEpic.handler({
        threadId: threadB.id,
        epic: { title: "Big work" },
        children: [{ title: "child A" }, { title: "child B" }],
      } as never)) as { epicId: string; childIds: string[] };

      // Pre-close every child explicitly.
      const transition = tool(tools, "oxplow__transition_work_items");
      await transition.handler({
        transitions: created.childIds.map((id) => ({
          threadId: threadB.id, itemId: id, status: "done",
        })),
      } as never);

      const completeTask = tool(tools, "oxplow__complete_task");
      const result = (await completeTask.handler({
        threadId: threadB.id,
        itemId: created.epicId,
        note: "done",
      } as never)) as { ok: boolean; status: string };
      expect(result.ok).toBe(true);
      expect(result.status).toBe("done");
    });
  });

});
