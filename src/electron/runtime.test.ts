import { describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { applyStatusTransition, buildThreadMcpConfig, buildRecentDoneReminder, buildPriorPromptInProgressReminder, buildSessionContextBlock, buildWikiCaptureHint, computeEffortFiles, describeHookHealth, isInsideWorktree, isReadIntentTool, isWriteIntentTool, shouldAcceptHookFilePath, terminalInputIsInterrupt } from "./runtime.js";
import { ThreadStore } from "../persistence/thread-store.js";
import { SnapshotStore } from "../persistence/snapshot-store.js";
import { StreamStore } from "../persistence/stream-store.js";
import { WorkItemEffortStore } from "../persistence/work-item-effort-store.js";
import { WorkItemStore } from "../persistence/work-item-store.js";
import type { McpServerHandle } from "../mcp/mcp-server.js";
import type { WorkItem, WorkItemKind, WorkItemPriority, WorkItemStatus } from "../persistence/work-item-store.js";
import type { ThreadStatus } from "../persistence/thread-store.js";

function workItem(id: string, status: WorkItemStatus, title = id): WorkItem {
  return {
    id,
    thread_id: "b1",
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
    author: null,
  };
}

function makeDoneItem(overrides: Partial<WorkItem>): WorkItem {
  return {
    ...workItem("wi-x", "done" as WorkItemStatus, "Some task"),
    author: "agent",
    ...overrides,
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

test("buildThreadMcpConfig points Claude at the shared HTTP MCP endpoint", () => {
  const config = JSON.parse(buildThreadMcpConfig(fakeMcp()));
  expect(config.mcpServers.oxplow).toEqual({
    type: "http",
    url: "http://127.0.0.1:43123/mcp",
    headers: {
      Authorization: "Bearer secret-token",
    },
  });
});

test("buildThreadMcpConfig only declares the oxplow server", () => {
  const config = JSON.parse(buildThreadMcpConfig(fakeMcp()));
  expect(Object.keys(config.mcpServers)).toEqual(["oxplow"]);
});

test("buildThreadMcpConfig embeds the exact bearer format", () => {
  const config = JSON.parse(buildThreadMcpConfig(fakeMcp({ authToken: "abc.def-ghi" })));
  expect(config.mcpServers.oxplow.headers.Authorization).toBe("Bearer abc.def-ghi");
});

test("buildThreadMcpConfig throws when the MCP server is not running", () => {
  expect(() => buildThreadMcpConfig(null)).toThrow("mcp server not started");
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

test("buildSessionContextBlock renders stream, thread, and writer distinction for a read-only thread", () => {
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Bugfixes" },
    thread: { id: "b-2", title: "Second" },
    activeThread: { id: "b-1", title: "Writer" },
  });
  expect(out).toContain("stream: \"Bugfixes\" (id: s-1)");
  expect(out).toContain("thread:  \"Second\" (id: b-2)");
  expect(out).toContain("writer: \"Writer\" (id: b-1) — your thread is read-only");
  expect(out).toMatch(/^<session-context>/);
  expect(out).toMatch(/<\/session-context>$/);
});

test("buildSessionContextBlock tells the agent it IS the writer when its thread matches activeThread", () => {
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    thread: { id: "b-1", title: "Default" },
    activeThread: { id: "b-1", title: "Default" },
  });
  expect(out).toContain("writer: (you)");
  expect(out).not.toContain("read-only");
});

test("buildSessionContextBlock treats a missing active thread as \"you're the writer\" (no active yet)", () => {
  // Rationale: the stores always return some activeThread today, but the
  // prompt shouldn't break if one day they don't. "You're the writer" is
  // the safe fallback — same behaviour as the pre-fix system prompt used.
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    thread: { id: "b-1", title: "Default" },
    activeThread: null,
  });
  expect(out).toContain("writer: (you)");
});

test("buildSessionContextBlock emits no ROLE CHANGE banner when initialRole matches current role", () => {
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    thread: { id: "b-1", title: "Default" },
    activeThread: { id: "b-1", title: "Default" },
    initialRole: "writer",
  });
  expect(out).not.toContain("ROLE CHANGE");
});

test("buildSessionContextBlock emits a read-only → writer ROLE CHANGE banner when the thread was read-only at session start and is now the writer", () => {
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    thread: { id: "b-2", title: "Mine" },
    activeThread: { id: "b-2", title: "Mine" },
    initialRole: "read-only",
  });
  expect(out).toContain("ROLE CHANGE");
  expect(out).toContain("this thread was read-only when the session started");
  expect(out).toContain("it is now the active writer");
  expect(out).toContain("NON_WRITER block in your initial system prompt is SUPERSEDED");
  // Banner sits before the closing tag.
  const bannerIdx = out.indexOf("ROLE CHANGE");
  const closeIdx = out.indexOf("</session-context>");
  expect(bannerIdx).toBeLessThan(closeIdx);
});

test("buildSessionContextBlock emits a writer → read-only ROLE CHANGE banner when the thread was the writer at session start and is now read-only", () => {
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    thread: { id: "b-2", title: "Mine" },
    activeThread: { id: "b-1", title: "Other" },
    initialRole: "writer",
  });
  expect(out).toContain("ROLE CHANGE");
  expect(out).toContain("this thread was the active writer when the session started");
  expect(out).toContain("it is now read-only");
  expect(out).toContain("NON_WRITER block applies now even though it wasn't in your initial system prompt");
});

test("buildSessionContextBlock omits ROLE CHANGE when initialRole is not supplied (backwards compatible)", () => {
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    thread: { id: "b-1", title: "Default" },
    activeThread: { id: "b-1", title: "Default" },
  });
  expect(out).not.toContain("ROLE CHANGE");
});


// ---- isInsideWorktree / shouldAcceptHookFilePath: hook path filtering ----

test("isInsideWorktree: absolute path inside the worktree is accepted", () => {
  const root = mkdtempSync(join(tmpdir(), "oxplow-runtime-"));
  try {
    expect(isInsideWorktree(resolve(root, "src/index.ts"), root)).toBe(true);
    expect(isInsideWorktree(root, root)).toBe(true);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("isInsideWorktree: path that resolves outside the worktree is rejected", () => {
  const parent = mkdtempSync(join(tmpdir(), "oxplow-runtime-"));
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
  const root = mkdtempSync(join(tmpdir(), "oxplow-runtime-"));
  try {
    expect(shouldAcceptHookFilePath(resolve(root, "src/index.ts"), root)).toBe(true);
    // Works with a relative path too.
    expect(shouldAcceptHookFilePath("src/index.ts", root)).toBe(true);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("shouldAcceptHookFilePath: rejects in-tree paths that match workspace ignore rules", () => {
  const root = mkdtempSync(join(tmpdir(), "oxplow-runtime-"));
  try {
    expect(shouldAcceptHookFilePath(resolve(root, ".oxplow/state.db"), root)).toBe(false);
    expect(shouldAcceptHookFilePath(resolve(root, "node_modules/x/index.js"), root)).toBe(false);
    expect(shouldAcceptHookFilePath(resolve(root, ".context/foo.md.tmp.1.2"), root)).toBe(false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("shouldAcceptHookFilePath: rejects paths that resolve outside the worktree", () => {
  const parent = mkdtempSync(join(tmpdir(), "oxplow-runtime-"));
  try {
    const root = join(parent, "worktree");
    expect(shouldAcceptHookFilePath("/tmp/elsewhere/file.ts", root)).toBe(false);
    expect(shouldAcceptHookFilePath("../escaped.ts", root)).toBe(false);
  } finally {
    rmSync(parent, { recursive: true, force: true });
  }
});

test("buildRecentDoneReminder: points at a recent agent-authored done item with the reopen instructions", () => {
  const now = Date.parse("2024-05-01T12:00:00Z");
  const recent = makeDoneItem({
    id: "wi-abc",
    title: "Wire up the paste handler",
    updated_at: "2024-05-01T11:55:00Z", // 5 min ago
  });
  const out = buildRecentDoneReminder([recent], now);
  expect(out).toContain("<recent-done-reminder>");
  expect(out).toContain("wi-abc");
  expect(out).toContain("Wire up the paste handler");
  expect(out).toContain("update_work_item");
  expect(out).toContain("in_progress");
  expect(out).toContain("Do NOT file a new");
});

test("buildRecentDoneReminder: ignores user-authored done items", () => {
  const now = Date.parse("2024-05-01T12:00:00Z");
  const userItem = makeDoneItem({
    id: "wi-user",
    author: "user",
    updated_at: "2024-05-01T11:55:00Z",
  });
  expect(buildRecentDoneReminder([userItem], now)).toBe("");
});

test("buildRecentDoneReminder: ignores items older than the window", () => {
  const now = Date.parse("2024-05-01T12:00:00Z");
  const stale = makeDoneItem({
    id: "wi-old",
    updated_at: "2024-05-01T10:00:00Z", // 2h ago, outside 15-min window
  });
  expect(buildRecentDoneReminder([stale], now)).toBe("");
});

test("buildRecentDoneReminder: ignores items not in done", () => {
  const now = Date.parse("2024-05-01T12:00:00Z");
  const inProgress = makeDoneItem({
    id: "wi-ip",
    status: "in_progress" as WorkItemStatus,
    updated_at: "2024-05-01T11:58:00Z",
  });
  expect(buildRecentDoneReminder([inProgress], now)).toBe("");
});

test("buildRecentDoneReminder: picks the most recent when multiple eligible items exist", () => {
  const now = Date.parse("2024-05-01T12:00:00Z");
  const older = makeDoneItem({ id: "wi-older", updated_at: "2024-05-01T11:50:00Z" });
  const newer = makeDoneItem({ id: "wi-newer", updated_at: "2024-05-01T11:58:00Z" });
  const out = buildRecentDoneReminder([older, newer], now);
  expect(out).toContain("wi-newer");
  expect(out).not.toContain("wi-older");
});

test("buildPriorPromptInProgressReminder: points at an in_progress item with the new-ask instructions", () => {
  const now = Date.parse("2024-05-01T12:00:00Z");
  const open = makeDoneItem({
    id: "wi-open",
    title: "Wire up the paste handler",
    status: "in_progress" as WorkItemStatus,
    updated_at: "2024-05-01T11:50:00Z",
  });
  const out = buildPriorPromptInProgressReminder([open], now);
  expect(out).toContain("<prior-prompt-in-progress-reminder>");
  expect(out).toContain("Wire up the paste handler");
  expect(out).toContain("create_work_item");
  expect(out).toContain("update_work_item");
  expect(out).toContain("Bundling a new ask");
});

test("buildPriorPromptInProgressReminder: empty when no in_progress items exist", () => {
  const now = Date.parse("2024-05-01T12:00:00Z");
  const done = makeDoneItem({ id: "wi-done", updated_at: "2024-05-01T11:55:00Z" });
  expect(buildPriorPromptInProgressReminder([done], now)).toBe("");
});

test("buildPriorPromptInProgressReminder: picks the most-recently-touched in_progress item", () => {
  const now = Date.parse("2024-05-01T12:00:00Z");
  const older = makeDoneItem({
    id: "wi-older",
    status: "in_progress" as WorkItemStatus,
    updated_at: "2024-05-01T10:00:00Z",
  });
  const newer = makeDoneItem({
    id: "wi-newer",
    status: "in_progress" as WorkItemStatus,
    updated_at: "2024-05-01T11:30:00Z",
  });
  const out = buildPriorPromptInProgressReminder([older, newer], now);
  expect(out).toContain("wi-newer");
  expect(out).not.toContain("wi-older");
});

test("decideStopDirective (via stop-hook-pipeline): empty thread allows stop", async () => {
  const { decideStopDirective } = await import("./stop-hook-pipeline.js");
  const out = decideStopDirective(
    {
      thread: {
        id: "b1", stream_id: "s1", title: "B", status: "active" as ThreadStatus, sort_index: 0,
        pane_target: "p", resume_session_id: "", custom_prompt: null, closed_at: null,
        created_at: "2024-01-01T00:00:00Z", updated_at: "2024-01-01T00:00:00Z",
      },
      workItems: [],
    },
    {},
  );
  expect(out.directive).toBeNull();
});

function seedHistoryHarness() {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-rth-"));
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
  const effortStore = new WorkItemEffortStore(dir);
  const snapshotStore = new SnapshotStore(dir);
  // Mirror production's workspace-watch ignore: skip .oxplow and common
  // build/cache dirs so the full-walk comparison is over the user's files
  // only. Without this, a full-walk flush would pick up the SQLite DB and
  // snapshot blobs, none of which are stable between calls.
  const ignore = (rel: string) => rel.startsWith(".oxplow") || rel.startsWith(".git");
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
  return { dir, stream, streamStore, threadStore, threadId, workItems, effortStore, snapshotStore, flushSnapshot };
}

describe("history-tracking runtime wiring", () => {
  test("status → in_progress opens an effort with a start_snapshot_id", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "T",
      createdBy: "user", actorId: "test",
    });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    applyStatusTransition(
      { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot },
      { threadId: h.threadId, workItemId: item.id, previous: "ready", next: "in_progress" },
    );
    const open = h.effortStore.getOpenEffort(item.id);
    expect(open).not.toBeNull();
    expect(open!.start_snapshot_id).not.toBeNull();
    expect(open!.ended_at).toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("in_progress → done closes the effort with an end_snapshot_id", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "T",
      createdBy: "user", actorId: "test",
    });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    applyStatusTransition(
      { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot },
      { threadId: h.threadId, workItemId: item.id, previous: "ready", next: "in_progress" },
    );
    writeFileSync(join(h.dir, "a.txt"), "v2");
    applyStatusTransition(
      { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot },
      { threadId: h.threadId, workItemId: item.id, previous: "in_progress", next: "done" },
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
      threadId: h.threadId, kind: "task", title: "T",
      createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot };
    writeFileSync(join(h.dir, "a.txt"), "v1");
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: item.id, previous: "ready", next: "in_progress" });
    writeFileSync(join(h.dir, "a.txt"), "v2");
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: item.id, previous: "in_progress", next: "done" });
    writeFileSync(join(h.dir, "a.txt"), "v3");
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: item.id, previous: "done", next: "ready" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: item.id, previous: "ready", next: "in_progress" });
    expect(h.effortStore.listEffortsForWorkItem(item.id)).toHaveLength(2);
    expect(h.effortStore.getOpenEffort(item.id)).not.toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("status changes with same-status or undefined-next are no-ops", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "T",
      createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: item.id, previous: "ready", next: undefined });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: item.id, previous: "in_progress", next: "in_progress" });
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
      ignore: (rel) => rel.startsWith(".oxplow") || rel.startsWith(".git"),
    });
    expect(again.created).toBe(false);
    rmSync(h.dir, { recursive: true, force: true });
  });
});

describe("touchedFiles payload on done transition", () => {
  test("update_work_item with touchedFiles populates work_item_effort_file on transition to done", () => {
    const h = seedHistoryHarness();
    const a = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "A", createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    const openEffort = h.effortStore.getOpenEffort(a.id)!;
    writeFileSync(join(h.dir, "a.txt"), "v1");
    writeFileSync(join(h.dir, "b.txt"), "v1");
    applyStatusTransition(deps, {
      threadId: h.threadId, workItemId: a.id,
      previous: "in_progress", next: "done",
      touchedFiles: ["a.txt", "b.txt", "a.txt"],
    });
    expect(h.effortStore.listEffortFiles(openEffort.id)).toEqual(["a.txt", "b.txt"]);
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("transition to blocked attaches touchedFiles (agent signalling handoff with its edits)", () => {
    const h = seedHistoryHarness();
    const a = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "A", createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    const openEffort = h.effortStore.getOpenEffort(a.id)!;
    applyStatusTransition(deps, {
      threadId: h.threadId, workItemId: a.id,
      previous: "in_progress", next: "blocked",
      touchedFiles: ["a.txt"],
    });
    expect(h.effortStore.listEffortFiles(openEffort.id)).toEqual(["a.txt"]);
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("transition to ready ignores touchedFiles (reopen path, no close attribution)", () => {
    const h = seedHistoryHarness();
    const a = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "A", createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    const openEffort = h.effortStore.getOpenEffort(a.id)!;
    applyStatusTransition(deps, {
      threadId: h.threadId, workItemId: a.id,
      previous: "in_progress", next: "ready",
      touchedFiles: ["b.txt"],
    });
    expect(h.effortStore.listEffortFiles(openEffort.id)).toEqual([]);
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("touchedFiles with >100 paths inserts nothing (server cap)", () => {
    const h = seedHistoryHarness();
    const a = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "A", createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    const openEffort = h.effortStore.getOpenEffort(a.id)!;
    const many: string[] = [];
    for (let i = 0; i < 101; i++) many.push(`f${i}.txt`);
    applyStatusTransition(deps, {
      threadId: h.threadId, workItemId: a.id,
      previous: "in_progress", next: "done",
      touchedFiles: many,
    });
    expect(h.effortStore.listEffortFiles(openEffort.id)).toEqual([]);
    rmSync(h.dir, { recursive: true, force: true });
  });
});

describe("computeEffortFiles", () => {
  test("single-effort snapshot returns raw pair-diff even with empty write log", () => {
    const h = seedHistoryHarness();
    const a = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "A", createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    writeFileSync(join(h.dir, "b.txt"), "v1");
    // No recordEffortFile calls — log is empty for this effort.
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "in_progress", next: "done" });
    const open = h.effortStore.listEffortsForWorkItem(a.id)[0]!;
    const summary = computeEffortFiles(h.effortStore, h.snapshotStore, open.id);
    expect(summary).not.toBeNull();
    // Raw pair-diff: both files show because they changed in the window.
    const paths = Object.keys(summary!.files).sort();
    expect(paths).toContain("a.txt");
    expect(paths).toContain("b.txt");
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("two efforts ending at same snapshot filter by the write log", () => {
    const h = seedHistoryHarness();
    const a = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "A", createdBy: "user", actorId: "test",
    });
    const b = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "B", createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot };
    // Both transitioning to in_progress, then each writes distinct files.
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: b.id, previous: "ready", next: "in_progress" });
    const effA = h.effortStore.getOpenEffort(a.id)!;
    const effB = h.effortStore.getOpenEffort(b.id)!;
    writeFileSync(join(h.dir, "a.txt"), "by-a");
    h.effortStore.recordEffortFile(effA.id, "a.txt");
    writeFileSync(join(h.dir, "b.txt"), "by-b");
    h.effortStore.recordEffortFile(effB.id, "b.txt");
    // Both efforts close at the same flush point (end-snapshot is shared).
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "in_progress", next: "done" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: b.id, previous: "in_progress", next: "done" });

    const closedA = h.effortStore.listEffortsForWorkItem(a.id)[0]!;
    const closedB = h.effortStore.listEffortsForWorkItem(b.id)[0]!;
    // When both end at the same snapshot the filter engages. If the two
    // task-end flushes happen to produce two snapshots (different version
    // hashes because of b.txt appearing between them), the filter won't
    // engage — so we only assert the filtering behaviour when the end ids
    // collide.
    if (closedA.end_snapshot_id === closedB.end_snapshot_id) {
      const sumA = computeEffortFiles(h.effortStore, h.snapshotStore, closedA.id);
      const sumB = computeEffortFiles(h.effortStore, h.snapshotStore, closedB.id);
      expect(Object.keys(sumA!.files)).toEqual(["a.txt"]);
      expect(Object.keys(sumB!.files)).toEqual(["b.txt"]);
    }
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("overlap: same file written by both efforts appears in both filtered results", () => {
    const h = seedHistoryHarness();
    const a = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "A", createdBy: "user", actorId: "test",
    });
    const b = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "B", createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: b.id, previous: "ready", next: "in_progress" });
    const effA = h.effortStore.getOpenEffort(a.id)!;
    const effB = h.effortStore.getOpenEffort(b.id)!;
    writeFileSync(join(h.dir, "shared.txt"), "final");
    h.effortStore.recordEffortFile(effA.id, "shared.txt");
    h.effortStore.recordEffortFile(effB.id, "shared.txt");
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "in_progress", next: "done" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: b.id, previous: "in_progress", next: "done" });
    const closedA = h.effortStore.listEffortsForWorkItem(a.id)[0]!;
    const closedB = h.effortStore.listEffortsForWorkItem(b.id)[0]!;
    if (closedA.end_snapshot_id === closedB.end_snapshot_id) {
      const sumA = computeEffortFiles(h.effortStore, h.snapshotStore, closedA.id);
      const sumB = computeEffortFiles(h.effortStore, h.snapshotStore, closedB.id);
      expect(Object.keys(sumA!.files)).toContain("shared.txt");
      expect(Object.keys(sumB!.files)).toContain("shared.txt");
    }
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("two efforts ending at same snapshot with no write log fall back to raw pair-diff", () => {
    const h = seedHistoryHarness();
    const a = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "A", createdBy: "user", actorId: "test",
    });
    const b = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "B", createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: b.id, previous: "ready", next: "in_progress" });
    writeFileSync(join(h.dir, "a.txt"), "by-a");
    writeFileSync(join(h.dir, "b.txt"), "by-b");
    // Neither transition declares touchedFiles — effort_file log stays empty.
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "in_progress", next: "done" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: b.id, previous: "in_progress", next: "done" });
    const closedA = h.effortStore.listEffortsForWorkItem(a.id)[0]!;
    const closedB = h.effortStore.listEffortsForWorkItem(b.id)[0]!;
    if (closedA.end_snapshot_id === closedB.end_snapshot_id) {
      const sumA = computeEffortFiles(h.effortStore, h.snapshotStore, closedA.id);
      const sumB = computeEffortFiles(h.effortStore, h.snapshotStore, closedB.id);
      // Assume-all fallback: both efforts report the union of changes.
      const pathsA = Object.keys(sumA!.files).sort();
      const pathsB = Object.keys(sumB!.files).sort();
      expect(pathsA).toContain("a.txt");
      expect(pathsA).toContain("b.txt");
      expect(pathsB).toContain("a.txt");
      expect(pathsB).toContain("b.txt");
    }
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("open effort (no end snapshot) returns null", () => {
    const h = seedHistoryHarness();
    const a = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "A", createdBy: "user", actorId: "test",
    });
    applyStatusTransition(
      { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot },
      { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" },
    );
    const effA = h.effortStore.getOpenEffort(a.id)!;
    expect(computeEffortFiles(h.effortStore, h.snapshotStore, effA.id)).toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });
});


describe("isWriteIntentTool", () => {
  test("Write/Edit/MultiEdit/NotebookEdit are always write-intent", () => {
    expect(isWriteIntentTool("Write", null)).toBe(true);
    expect(isWriteIntentTool("Edit", null)).toBe(true);
    expect(isWriteIntentTool("MultiEdit", null)).toBe(true);
    expect(isWriteIntentTool("NotebookEdit", null)).toBe(true);
  });
  test("read-only tools aren't write-intent", () => {
    expect(isWriteIntentTool("Read", null)).toBe(false);
    expect(isWriteIntentTool("Grep", null)).toBe(false);
  });
  test("Bash with read-only command is not write-intent", () => {
    expect(isWriteIntentTool("Bash", { command: "ls -la" })).toBe(false);
    expect(isWriteIntentTool("Bash", { command: "cat src/foo.ts" })).toBe(false);
    expect(isWriteIntentTool("Bash", { command: "git status" })).toBe(false);
    expect(isWriteIntentTool("Bash", { command: "bun test src/foo.test.ts" })).toBe(false);
    expect(isWriteIntentTool("Bash", { command: "bunx tsc --noEmit" })).toBe(false);
  });
  test("git commit / git push are NOT write-intent — the work being committed was already tracked", () => {
    // Filing-enforcement Stop branch fires on write-intent. Counting a
    // `git commit` as write-intent forces the agent to file a placeholder
    // "Commit XYZ" item just to attribute the Bash invocation, which is
    // bookkeeping noise for already-tracked work.
    expect(isWriteIntentTool("Bash", { command: "git commit -m 'ship it'" })).toBe(false);
    expect(isWriteIntentTool("Bash", { command: "git push origin main" })).toBe(false);
    expect(isWriteIntentTool("Bash", { command: "git push" })).toBe(false);
  });
  test("other git mutating commands stay write-intent", () => {
    // git rm / git checkout / git reset / git rebase still rewrite the
    // working tree or refs in ways the Work panel SHOULD track.
    expect(isWriteIntentTool("Bash", { command: "git rm src/foo.ts" })).toBe(true);
    expect(isWriteIntentTool("Bash", { command: "git checkout -- src/foo.ts" })).toBe(true);
    expect(isWriteIntentTool("Bash", { command: "git reset --hard origin/main" })).toBe(true);
  });
  test("Bash with other/unknown command is write-intent (err toward auto-file)", () => {
    expect(isWriteIntentTool("Bash", { command: "npm install foo" })).toBe(true);
    expect(isWriteIntentTool("Bash", { command: "rm -rf dist" })).toBe(true);
  });
});

describe("isReadIntentTool", () => {
  test("Read/Grep/Glob always count as reads", () => {
    expect(isReadIntentTool("Read", null)).toBe(true);
    expect(isReadIntentTool("Grep", null)).toBe(true);
    expect(isReadIntentTool("Glob", null)).toBe(true);
  });
  test("write-intent tools don't count as reads", () => {
    expect(isReadIntentTool("Write", null)).toBe(false);
    expect(isReadIntentTool("Edit", null)).toBe(false);
  });
  test("read-only Bash counts as a read", () => {
    expect(isReadIntentTool("Bash", { command: "ls -la" })).toBe(true);
    expect(isReadIntentTool("Bash", { command: "git diff" })).toBe(true);
    expect(isReadIntentTool("Bash", { command: "bun test foo" })).toBe(true);
  });
  test("git commit / git push count as reads (activity but not write-intent)", () => {
    // The "not write-intent" rule pulls them into the read column so a
    // commit-only turn still counts as activity (no Q&A short-circuit)
    // but doesn't trip filing-enforcement.
    expect(isReadIntentTool("Bash", { command: "git commit -m 'foo'" })).toBe(true);
    expect(isReadIntentTool("Bash", { command: "git push origin main" })).toBe(true);
  });
  test("write-intent Bash doesn't count as a read", () => {
    expect(isReadIntentTool("Bash", { command: "rm -rf dist" })).toBe(false);
    expect(isReadIntentTool("Bash", { command: "npm install" })).toBe(false);
  });
  test("MCP / other tool names don't count as reads or writes (out-of-scope)", () => {
    expect(isReadIntentTool("mcp__oxplow__list_notes", null)).toBe(false);
    expect(isReadIntentTool("Task", null)).toBe(false);
  });
});

describe("buildWikiCaptureHint", () => {
  test("returns a hint block for exploration prompts", () => {
    for (const prompt of [
      "how does the stop hook work?",
      "explain the wiki note storage",
      "describe the architecture",
      "give me an overview of the code",
      "walk me through the runtime",
      "create a high level architecture of the code",
      "summarize the codebase",
      "what is the architecture of the snapshot store",
      "trace how a UserPromptSubmit hook flows through the runtime",
      "where is the write guard implemented",
    ]) {
      const text = buildWikiCaptureHint(prompt);
      expect(text, `expected hint for: ${prompt}`).not.toBeNull();
      expect(text!).toContain("<wiki-capture-hint>");
      expect(text!).toContain("synthesis / exploration");
      expect(text!).toContain(".oxplow/notes/");
      expect(text!).toContain("resync_note");
    }
  });
  test("returns null for non-exploration prompts", () => {
    for (const prompt of [
      "fix the login redirect bug",
      "add a delete button to the work panel",
      "refactor the snapshot store",
      "yes, proceed",
      "rename foo to bar",
      "",
      "   ",
    ]) {
      expect(buildWikiCaptureHint(prompt), `expected null for: ${prompt}`).toBeNull();
    }
  });
});


describe("terminalInputIsInterrupt", () => {
  const b64 = (s: string) => Buffer.from(s, "binary").toString("base64");
  test("bare ESC byte is an interrupt", () => {
    expect(terminalInputIsInterrupt(JSON.stringify({ type: "input", bytes: b64("\x1b") }))).toBe(true);
  });
  test("Ctrl-C byte is an interrupt", () => {
    expect(terminalInputIsInterrupt(JSON.stringify({ type: "input", bytes: b64("\x03") }))).toBe(true);
  });
  test("arrow key (ESC sequence) is NOT an interrupt", () => {
    // \x1b[A — up arrow. Multi-byte ESC sequences must not match.
    expect(terminalInputIsInterrupt(JSON.stringify({ type: "input", bytes: b64("\x1b[A") }))).toBe(false);
  });
  test("printable text is NOT an interrupt", () => {
    expect(terminalInputIsInterrupt(JSON.stringify({ type: "input", bytes: b64("hello") }))).toBe(false);
  });
  test("non-input messages are NOT an interrupt", () => {
    expect(terminalInputIsInterrupt(JSON.stringify({ type: "resize", cols: 80, rows: 24 }))).toBe(false);
    expect(terminalInputIsInterrupt(JSON.stringify({ type: "history-exit" }))).toBe(false);
  });
  test("malformed JSON is NOT an interrupt", () => {
    expect(terminalInputIsInterrupt("not json")).toBe(false);
    expect(terminalInputIsInterrupt("")).toBe(false);
  });
  test("input-binary with bare ESC is also recognized", () => {
    expect(terminalInputIsInterrupt(JSON.stringify({ type: "input-binary", bytes: b64("\x1b") }))).toBe(true);
  });
});
