import { describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { applyStatusTransition, buildAutoCommitMessage, buildBatchMcpConfig, buildNextWorkItemStopReason, buildSessionContextBlock, describeHookHealth, isInsideWorktree, linkOpenEffortsToTurn, shouldAcceptHookFilePath } from "./runtime.js";
import { BatchStore } from "../persistence/batch-store.js";
import { SnapshotStore } from "../persistence/snapshot-store.js";
import { StreamStore } from "../persistence/stream-store.js";
import { TurnStore } from "../persistence/turn-store.js";
import { WorkItemEffortStore } from "../persistence/work-item-effort-store.js";
import { WorkItemStore } from "../persistence/work-item-store.js";
import type { McpServerHandle } from "../mcp/mcp-server.js";
import type { WorkItem, WorkItemKind, WorkItemPriority, WorkItemStatus } from "../persistence/work-item-store.js";
import type { BatchStatus } from "../persistence/batch-store.js";

function workItem(id: string, status: WorkItemStatus, title = id): WorkItem {
  return {
    id,
    batch_id: "b1",
    parent_id: null,
    kind: "task" as WorkItemKind,
    title,
    description: "",
    acceptance_criteria: null,
    status,
    priority: "medium" as WorkItemPriority,
    sort_index: 0,
    created_by: "user",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    completed_at: null,
    deleted_at: null,
    note_count: 0,
  };
}

function fakeMcp(overrides: Partial<McpServerHandle> = {}): McpServerHandle {
  return {
    port: 43123,
    authToken: "secret-token",
    httpUrl: "http://127.0.0.1:43123/mcp",
    hookUrl: "http://127.0.0.1:43123/hook",
    lockfilePath: "/tmp/43123.lock",
    stop: async () => {},
    ...overrides,
  };
}

test("buildBatchMcpConfig points Claude at the shared HTTP MCP endpoint", () => {
  const config = JSON.parse(buildBatchMcpConfig(fakeMcp()));
  expect(config.mcpServers.newde).toEqual({
    type: "http",
    url: "http://127.0.0.1:43123/mcp",
    headers: {
      Authorization: "Bearer secret-token",
    },
  });
});

test("buildBatchMcpConfig only declares the newde server", () => {
  const config = JSON.parse(buildBatchMcpConfig(fakeMcp()));
  expect(Object.keys(config.mcpServers)).toEqual(["newde"]);
});

test("buildBatchMcpConfig embeds the exact bearer format", () => {
  const config = JSON.parse(buildBatchMcpConfig(fakeMcp({ authToken: "abc.def-ghi" })));
  expect(config.mcpServers.newde.headers.Authorization).toBe("Bearer abc.def-ghi");
});

test("buildBatchMcpConfig throws when the MCP server is not running", () => {
  expect(() => buildBatchMcpConfig(null)).toThrow("mcp server not started");
});

test("describeHookHealth emits nothing when every registered hook delivered", () => {
  const seen = new Set(["PreToolUse", "PostToolUse", "UserPromptSubmit", "SessionStart", "SessionEnd", "Stop", "Notification"]);
  expect(describeHookHealth(["PreToolUse", "SessionStart", "Stop"], seen)).toEqual([]);
});

test("describeHookHealth calls out SessionStart with the known-workaround hint", () => {
  const reports = describeHookHealth(["SessionStart", "Stop"], new Set(["Stop"]));
  expect(reports).toHaveLength(1);
  expect(reports[0]!.event).toBe("SessionStart");
  expect(reports[0]!.message).toBe("registered hook never delivered");
  expect(reports[0]!.hint).toMatch(/Claude Code drops HTTP hooks for SessionStart/);
});

test("describeHookHealth emits the generic hint for other undelivered hooks", () => {
  const reports = describeHookHealth(["PreToolUse", "Notification"], new Set());
  expect(reports.map((r) => r.event)).toEqual(["PreToolUse", "Notification"]);
  for (const report of reports) {
    expect(report.message).toBe("registered hook not observed yet");
    expect(report.hint).toMatch(/Expected if the turn didn't exercise/);
  }
});

test("buildSessionContextBlock renders stream, batch, and writer distinction for a read-only batch", () => {
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Bugfixes" },
    batch: { id: "b-2", title: "Second" },
    activeBatch: { id: "b-1", title: "Writer" },
  });
  expect(out).toContain("stream: \"Bugfixes\" (id: s-1)");
  expect(out).toContain("batch:  \"Second\" (id: b-2)");
  expect(out).toContain("writer: \"Writer\" (id: b-1) — your batch is read-only");
  expect(out).toMatch(/^<session-context>/);
  expect(out).toMatch(/<\/session-context>$/);
});

test("buildSessionContextBlock tells the agent it IS the writer when its batch matches activeBatch", () => {
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    batch: { id: "b-1", title: "Default" },
    activeBatch: { id: "b-1", title: "Default" },
  });
  expect(out).toContain("writer: (you)");
  expect(out).not.toContain("read-only");
});

test("buildSessionContextBlock treats a missing active batch as \"you're the writer\" (no active yet)", () => {
  // Rationale: the stores always return some activeBatch today, but the
  // prompt shouldn't break if one day they don't. "You're the writer" is
  // the safe fallback — same behaviour as the pre-fix system prompt used.
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    batch: { id: "b-1", title: "Default" },
    activeBatch: null,
  });
  expect(out).toContain("writer: (you)");
});

test("buildNextWorkItemStopReason prepends a UI-change nudge banner when context.uiChangeNudge is true", () => {
  const text = buildNextWorkItemStopReason(
    { id: "wi-x", title: "do", kind: "task", batch_id: "b-y" },
    "s-z",
    { uiChangeNudge: true },
  );
  expect(text).toMatch(/^⚠ UI change detected/);
  expect(text).toMatch(/restart newde/i);
  expect(text).toContain("exercise the feature in the browser");
  // The dispatch body still follows after the banner.
  expect(text).toContain("read_work_options");
});

test("buildNextWorkItemStopReason omits the nudge banner when uiChangeNudge is false / absent", () => {
  const text = buildNextWorkItemStopReason(
    { id: "wi-x", title: "do", kind: "task", batch_id: "b-y" },
    "s-z",
    {},
  );
  expect(text).not.toContain("UI change detected");
  expect(text).toMatch(/^The batch queue has ready work/);
});

test("buildNextWorkItemStopReason directs the agent to call read_work_options and dispatch a subagent", () => {
  const text = buildNextWorkItemStopReason(
    { id: "wi-abc", title: "Do the thing", kind: "task", batch_id: "b-xyz" },
    "s-123",
  );
  expect(text).toContain("read_work_options");
  expect(text).toContain("general-purpose");
  // batch_id is embedded in the read_work_options call so the agent can pass the right batchId.
  expect(text).toMatch(/batchId="b-xyz"/);
  // Protocol details (one-at-a-time attribution, human_check, etc.) live in the
  // newde-task-management skill — the directive just points at it to stay terse.
  expect(text).toContain("newde-task-management");
  // Trimmed: directive should be a single line / well under 400 tokens.
  expect(text.length).toBeLessThan(400);
});

// ---- buildAutoCommitMessage: auto-mode commit point message generation ----

test("buildAutoCommitMessage: no settled work → fallback message", () => {
  const msg = buildAutoCommitMessage([workItem("w1", "ready"), workItem("w2", "in_progress")]);
  expect(msg).toBe("chore: auto-commit at queue commit point");
});

test("buildAutoCommitMessage: single settled item → single-item conventional-commit", () => {
  const msg = buildAutoCommitMessage([workItem("w1", "human_check", "Fix login bug")]);
  expect(msg).toBe("chore: Fix login bug");
});

test("buildAutoCommitMessage: multiple settled items → multi-item summary", () => {
  const items = [
    workItem("w1", "done", "Add user auth"),
    workItem("w2", "human_check", "Fix login bug"),
  ];
  const msg = buildAutoCommitMessage(items);
  expect(msg).toMatch(/^chore: complete 2 work items/);
  expect(msg).toContain("- Add user auth");
  expect(msg).toContain("- Fix login bug");
});

test("buildAutoCommitMessage: canceled items count as settled", () => {
  const msg = buildAutoCommitMessage([workItem("w1", "canceled", "Cancelled task")]);
  expect(msg).toBe("chore: Cancelled task");
});

test("buildAutoCommitMessage: more than 5 settled items truncates with ellipsis", () => {
  const items = Array.from({ length: 7 }, (_, i) =>
    workItem(`w${i}`, "done", `Task ${i + 1}`),
  );
  const msg = buildAutoCommitMessage(items);
  expect(msg).toContain("…and 2 more");
  // Only first 5 listed.
  expect(msg).toContain("- Task 1");
  expect(msg).toContain("- Task 5");
  expect(msg).not.toContain("- Task 6");
});

// ---- stop-hook pipeline: approve-mode commit point blocks; auto-mode is handled by runtime before pipeline ----

// ---- isInsideWorktree / shouldAcceptHookFilePath: hook path filtering ----

test("isInsideWorktree: absolute path inside the worktree is accepted", () => {
  const root = mkdtempSync(join(tmpdir(), "newde-runtime-"));
  try {
    expect(isInsideWorktree(resolve(root, "src/index.ts"), root)).toBe(true);
    expect(isInsideWorktree(root, root)).toBe(true);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("isInsideWorktree: path that resolves outside the worktree is rejected", () => {
  const parent = mkdtempSync(join(tmpdir(), "newde-runtime-"));
  try {
    const root = join(parent, "worktree");
    // ../ escape from within root
    expect(isInsideWorktree("../escaped.ts", root)).toBe(false);
    // Entirely different absolute path
    expect(isInsideWorktree("/etc/passwd", root)).toBe(false);
  } finally {
    rmSync(parent, { recursive: true, force: true });
  }
});

test("shouldAcceptHookFilePath: accepts a normal in-tree source file", () => {
  const root = mkdtempSync(join(tmpdir(), "newde-runtime-"));
  try {
    expect(shouldAcceptHookFilePath(resolve(root, "src/index.ts"), root)).toBe(true);
    // Works with a relative path too.
    expect(shouldAcceptHookFilePath("src/index.ts", root)).toBe(true);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("shouldAcceptHookFilePath: rejects in-tree paths that match workspace ignore rules", () => {
  const root = mkdtempSync(join(tmpdir(), "newde-runtime-"));
  try {
    expect(shouldAcceptHookFilePath(resolve(root, ".newde/state.db"), root)).toBe(false);
    expect(shouldAcceptHookFilePath(resolve(root, "node_modules/x/index.js"), root)).toBe(false);
    expect(shouldAcceptHookFilePath(resolve(root, ".context/foo.md.tmp.1.2"), root)).toBe(false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("shouldAcceptHookFilePath: rejects paths that resolve outside the worktree", () => {
  const parent = mkdtempSync(join(tmpdir(), "newde-runtime-"));
  try {
    const root = join(parent, "worktree");
    expect(shouldAcceptHookFilePath("/tmp/elsewhere/file.ts", root)).toBe(false);
    expect(shouldAcceptHookFilePath("../escaped.ts", root)).toBe(false);
  } finally {
    rmSync(parent, { recursive: true, force: true });
  }
});

test("decideStopDirective (via stop-hook-pipeline): approve-mode pending commit point blocks", async () => {
  // Import the pure pipeline function directly — no need for the full runtime.
  const { decideStopDirective } = await import("./stop-hook-pipeline.js");
  const { default: batchFactory } = await Promise.resolve({ default: (overrides = {}) => ({
    id: "b1", stream_id: "s1", title: "B", status: "active" as BatchStatus, sort_index: 0,
    pane_target: "p", resume_session_id: "", auto_commit: false, custom_prompt: null,
    created_at: "2024-01-01T00:00:00Z", updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  }) });
  const cp = {
    id: "cp1", batch_id: "b1", sort_index: 1, mode: "approve" as const,
    status: "pending" as const, commit_sha: null,
    created_at: "2024-01-01T00:00:00Z", updated_at: "2024-01-01T00:00:00Z", completed_at: null,
  };
  const out = decideStopDirective(
    { batch: batchFactory(), commitPoints: [cp], waitPoints: [], workItems: [], readyWorkItems: [] },
    { buildCommitPointReason: (c) => `commit: ${c.id}`, buildNextWorkItemReason: (i) => `next: ${i.id}` },
  );
  expect(out.directive?.decision).toBe("block");
  expect(out.directive?.reason).toContain("commit: cp1");
});

function seedHistoryHarness() {
  const dir = mkdtempSync(join(tmpdir(), "newde-rth-"));
  const streamStore = new StreamStore(dir);
  const stream = streamStore.create({
    title: "Demo",
    branch: "main",
    worktreePath: dir,
    projectBase: "demo",
  });
  const batchStore = new BatchStore(dir);
  const state = batchStore.ensureStream(stream);
  const batchId = state.batches[0]!.id;
  const workItems = new WorkItemStore(dir);
  const turnStore = new TurnStore(dir);
  const effortStore = new WorkItemEffortStore(dir);
  const snapshotStore = new SnapshotStore(dir);
  // Mirror production's workspace-watch ignore: skip .newde and common
  // build/cache dirs so the full-walk comparison is over the user's files
  // only. Without this, a full-walk flush would pick up the SQLite DB and
  // snapshot blobs, none of which are stable between calls.
  const ignore = (rel: string) => rel.startsWith(".newde") || rel.startsWith(".git");
  // Flush a baseline so the first task-start/turn-start snapshot has a
  // `getLatestSnapshot` to dedup against (mirrors real startup flow).
  writeFileSync(join(dir, "seed.txt"), "baseline");
  snapshotStore.flushSnapshot({
    source: "startup",
    streamId: stream.id,
    worktreePath: dir,
    dirtyPaths: null,
    ignore,
  });
  const flushSnapshot = (source: Parameters<typeof snapshotStore.flushSnapshot>[0]["source"]) => {
    const result = snapshotStore.flushSnapshot({
      source,
      streamId: stream.id,
      worktreePath: dir,
      dirtyPaths: null,
      ignore,
    });
    return result.id;
  };
  return { dir, stream, streamStore, batchStore, batchId, workItems, turnStore, effortStore, snapshotStore, flushSnapshot };
}

describe("history-tracking runtime wiring", () => {
  test("status → in_progress opens an effort with a start_snapshot_id", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      batchId: h.batchId, kind: "task", title: "T",
      createdBy: "user", actorId: "test",
    });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    applyStatusTransition(
      { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot },
      { batchId: h.batchId, workItemId: item.id, previous: "ready", next: "in_progress" },
    );
    const open = h.effortStore.getOpenEffort(item.id);
    expect(open).not.toBeNull();
    expect(open!.start_snapshot_id).not.toBeNull();
    expect(open!.ended_at).toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("in_progress → human_check closes the effort with an end_snapshot_id", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      batchId: h.batchId, kind: "task", title: "T",
      createdBy: "user", actorId: "test",
    });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    applyStatusTransition(
      { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot },
      { batchId: h.batchId, workItemId: item.id, previous: "ready", next: "in_progress" },
    );
    writeFileSync(join(h.dir, "a.txt"), "v2");
    applyStatusTransition(
      { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot },
      { batchId: h.batchId, workItemId: item.id, previous: "in_progress", next: "human_check" },
    );
    expect(h.effortStore.getOpenEffort(item.id)).toBeNull();
    const all = h.effortStore.listEffortsForWorkItem(item.id);
    expect(all).toHaveLength(1);
    expect(all[0]!.end_snapshot_id).not.toBeNull();
    expect(all[0]!.ended_at).not.toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("reopening a task creates a second independent effort", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      batchId: h.batchId, kind: "task", title: "T",
      createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    writeFileSync(join(h.dir, "a.txt"), "v1");
    applyStatusTransition(deps, { batchId: h.batchId, workItemId: item.id, previous: "ready", next: "in_progress" });
    writeFileSync(join(h.dir, "a.txt"), "v2");
    applyStatusTransition(deps, { batchId: h.batchId, workItemId: item.id, previous: "in_progress", next: "human_check" });
    writeFileSync(join(h.dir, "a.txt"), "v3");
    applyStatusTransition(deps, { batchId: h.batchId, workItemId: item.id, previous: "human_check", next: "ready" });
    applyStatusTransition(deps, { batchId: h.batchId, workItemId: item.id, previous: "ready", next: "in_progress" });
    expect(h.effortStore.listEffortsForWorkItem(item.id)).toHaveLength(2);
    expect(h.effortStore.getOpenEffort(item.id)).not.toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("opening a turn after an effort opens attaches it via linkEffortTurn", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      batchId: h.batchId, kind: "task", title: "T",
      createdBy: "user", actorId: "test",
    });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    applyStatusTransition(
      { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot },
      { batchId: h.batchId, workItemId: item.id, previous: "ready", next: "in_progress" },
    );
    const turn = h.turnStore.openTurn({ batchId: h.batchId, prompt: "P" });
    linkOpenEffortsToTurn(h.effortStore, turn.id);
    const effort = h.effortStore.getOpenEffort(item.id)!;
    expect(h.effortStore.listTurnsForEffort(effort.id)).toEqual([turn.id]);
    expect(h.effortStore.listEffortsForTurn(turn.id).map((e) => e.id)).toEqual([effort.id]);
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("effort opening during an already-open turn links back to that turn", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      batchId: h.batchId, kind: "task", title: "T",
      createdBy: "user", actorId: "test",
    });
    // Turn open first...
    const turn = h.turnStore.openTurn({ batchId: h.batchId, prompt: "P" });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    // ...then the task transitions to in_progress. applyStatusTransition
    // should see the open turn via currentOpenTurn and link itself.
    applyStatusTransition(
      { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot },
      { batchId: h.batchId, workItemId: item.id, previous: "ready", next: "in_progress" },
    );
    const effort = h.effortStore.getOpenEffort(item.id)!;
    expect(h.effortStore.listTurnsForEffort(effort.id)).toEqual([turn.id]);
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("status changes with same-status or undefined-next are no-ops", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      batchId: h.batchId, kind: "task", title: "T",
      createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { batchId: h.batchId, workItemId: item.id, previous: "ready", next: undefined });
    applyStatusTransition(deps, { batchId: h.batchId, workItemId: item.id, previous: "in_progress", next: "in_progress" });
    expect(h.effortStore.listEffortsForWorkItem(item.id)).toHaveLength(0);
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("startup snapshot on unchanged worktree dedups (no new row)", () => {
    const h = seedHistoryHarness();
    // Harness already flushed one baseline snapshot. A second flush with no
    // worktree changes should return the same id with `created: false`.
    const again = h.snapshotStore.flushSnapshot({
      source: "startup",
      streamId: h.stream.id,
      worktreePath: h.dir,
      dirtyPaths: null,
      ignore: (rel) => rel.startsWith(".newde") || rel.startsWith(".git"),
    });
    expect(again.created).toBe(false);
    rmSync(h.dir, { recursive: true, force: true });
  });
});
