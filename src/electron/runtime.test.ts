import { describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { applyStatusTransition, autoCompleteOpenAutoItems, autoFileWorkItemIfNeeded, AUTO_COMPLETE_NOTE_MAX_LEN, backfillCommitLinksForThread, buildAutoCommitMessage, buildAutoCommitStopReason, buildCommitPointStopReason, buildThreadMcpConfig, buildNextWorkItemStopReason, buildSessionContextBlock, composeAutoCompleteNote, composeTaskListNote, computeEffortFiles, deriveAutoItemTitleFromDiff, deriveAutoItemTitleFromPrompt, describeHookHealth, detectCommitShasFromBashOutput, detectTestResultFromBashOutput, detectTscErrorsFromBashOutput, extractBashStdout, extractDispatchedItemIds, extractTodoList, isDispatchLikeTool, isInsideWorktree, isWriteIntentTool, linkCommitToContributingItems, linkOpenEffortsToTurn, shouldAcceptHookFilePath } from "./runtime.js";
import { WorkItemCommitStore } from "../persistence/work-item-commit-store.js";
import { CommitPointStore } from "../persistence/commit-point-store.js";
import { ThreadStore } from "../persistence/thread-store.js";
import { SnapshotStore } from "../persistence/snapshot-store.js";
import { StreamStore } from "../persistence/stream-store.js";
import { TurnStore } from "../persistence/turn-store.js";
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
  expect(config.mcpServers.newde).toEqual({
    type: "http",
    url: "http://127.0.0.1:43123/mcp",
    headers: {
      Authorization: "Bearer secret-token",
    },
  });
});

test("buildThreadMcpConfig only declares the newde server", () => {
  const config = JSON.parse(buildThreadMcpConfig(fakeMcp()));
  expect(Object.keys(config.mcpServers)).toEqual(["newde"]);
});

test("buildThreadMcpConfig embeds the exact bearer format", () => {
  const config = JSON.parse(buildThreadMcpConfig(fakeMcp({ authToken: "abc.def-ghi" })));
  expect(config.mcpServers.newde.headers.Authorization).toBe("Bearer abc.def-ghi");
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

test("buildSessionContextBlock omits last_turn_cache_read line when no prior turn exists", () => {
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    thread: { id: "b-1", title: "Default" },
    activeThread: { id: "b-1", title: "Default" },
  });
  expect(out).not.toContain("last_turn_cache_read");
});

test("buildSessionContextBlock omits last_turn_cache_read line for tiny values (<1000)", () => {
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    thread: { id: "b-1", title: "Default" },
    activeThread: { id: "b-1", title: "Default" },
    lastTurnCacheRead: 500,
  });
  expect(out).not.toContain("last_turn_cache_read");
});

test("buildSessionContextBlock formats last_turn_cache_read in K for mid-sized values", () => {
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    thread: { id: "b-1", title: "Default" },
    activeThread: { id: "b-1", title: "Default" },
    lastTurnCacheRead: 120_000,
  });
  expect(out).toContain("last_turn_cache_read: 120K");
  expect(out).not.toContain("dispatch new work to subagents");
});

test("buildSessionContextBlock formats last_turn_cache_read in M for ≥1M and omits hint below 10M", () => {
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    thread: { id: "b-1", title: "Default" },
    activeThread: { id: "b-1", title: "Default" },
    lastTurnCacheRead: 6_600_000,
  });
  expect(out).toContain("last_turn_cache_read: 6.6M");
  expect(out).not.toContain("dispatch new work to subagents");
});

test("buildSessionContextBlock renders currentTurnBytes alongside last_turn_cache_read when provided", () => {
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    thread: { id: "b-1", title: "Default" },
    activeThread: { id: "b-1", title: "Default" },
    lastTurnCacheRead: 19_300_000,
    currentTurnBytes: 2_000_000,
  });
  expect(out).toContain("last_turn_cache_read: 19.3M (this turn: ~2.0M so far)");
});

test("buildSessionContextBlock omits this-turn suffix when currentTurnBytes is absent or tiny", () => {
  const outNone = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    thread: { id: "b-1", title: "Default" },
    activeThread: { id: "b-1", title: "Default" },
    lastTurnCacheRead: 19_300_000,
  });
  expect(outNone).toContain("last_turn_cache_read: 19.3M");
  expect(outNone).not.toContain("this turn:");

  const outTiny = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    thread: { id: "b-1", title: "Default" },
    activeThread: { id: "b-1", title: "Default" },
    lastTurnCacheRead: 19_300_000,
    currentTurnBytes: 500,
  });
  expect(outTiny).not.toContain("this turn:");
});

test("buildSessionContextBlock appends dispatch-to-subagent hint once last_turn_cache_read crosses 10M", () => {
  const out = buildSessionContextBlock({
    stream: { id: "s-1", title: "Main" },
    thread: { id: "b-1", title: "Default" },
    activeThread: { id: "b-1", title: "Default" },
    lastTurnCacheRead: 15_000_000,
  });
  expect(out).toContain("last_turn_cache_read: 15.0M");
  expect(out).toContain("dispatch new work to subagents");
  // Hint sits before the closing tag.
  const hintIdx = out.indexOf("dispatch new work to subagents");
  const closeIdx = out.indexOf("</session-context>");
  expect(hintIdx).toBeLessThan(closeIdx);
});

test("buildNextWorkItemStopReason prepends a UI-change nudge banner when context.uiChangeNudge is true", () => {
  const text = buildNextWorkItemStopReason(
    { id: "wi-x", title: "do", kind: "task", thread_id: "b-y" },
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
    { id: "wi-x", title: "do", kind: "task", thread_id: "b-y" },
    "s-z",
    {},
  );
  expect(text).not.toContain("UI change detected");
  expect(text).toMatch(/^The thread queue has ready work/);
});

test("buildNextWorkItemStopReason directs the agent to call read_work_options and dispatch a subagent", () => {
  const text = buildNextWorkItemStopReason(
    { id: "wi-abc", title: "Do the thing", kind: "task", thread_id: "b-xyz" },
    "s-123",
  );
  expect(text).toContain("read_work_options");
  expect(text).toContain("general-purpose");
  // thread_id is embedded in the read_work_options call so the agent can pass the right threadId.
  expect(text).toMatch(/threadId="b-xyz"/);
  // Protocol details (one-at-a-time attribution, human_check, etc.) live in the
  // merged newde-runtime skill — the directive just points at it to stay terse.
  expect(text).toContain("newde-runtime");
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

test("buildAutoCommitMessage: previousCommitCompletedAt filters out items updated before the prior commit", () => {
  // The monotonically-growing-count bug: items settled BEFORE the previous
  // commit re-appeared in every subsequent auto-commit message. Fix:
  // filter by `updated_at > previousCommitCompletedAt`.
  const older = { ...workItem("w1", "done", "Older done task"), updated_at: "2024-01-01T00:00:00Z" };
  const newer = { ...workItem("w2", "human_check", "Fresh settled task"), updated_at: "2024-02-01T00:00:00Z" };
  const msg = buildAutoCommitMessage([older, newer], "2024-01-15T00:00:00Z");
  expect(msg).toBe("chore: Fresh settled task");
  expect(msg).not.toContain("Older done task");
});

test("buildAutoCommitMessage: null previousCommitCompletedAt includes everything (first-commit case)", () => {
  const a = { ...workItem("w1", "done", "A"), updated_at: "2024-01-01T00:00:00Z" };
  const b = { ...workItem("w2", "human_check", "B"), updated_at: "2024-02-01T00:00:00Z" };
  const msg = buildAutoCommitMessage([a, b], null);
  expect(msg).toMatch(/complete 2 work items/);
});

test("buildAutoCommitMessage: no items survive the filter → fallback text", () => {
  const older = { ...workItem("w1", "done", "Ancient"), updated_at: "2024-01-01T00:00:00Z" };
  const msg = buildAutoCommitMessage([older], "2024-02-01T00:00:00Z");
  expect(msg).toBe("chore: auto-commit at queue commit point");
});

test("buildAutoCommitStopReason: ad-hoc (cp=null) directive asks for auto shape without commit_point_id", () => {
  const text = buildAutoCommitStopReason(null);
  expect(text).toMatch(/auto: true, message/);
  expect(text).not.toMatch(/commit_point_id/);
  expect(text).toContain("tasks_since_last_commit");
  // No approval gate: should explicitly forbid asking, not request it.
  expect(text).toMatch(/do NOT ask the user to approve/i);
  expect(text).not.toMatch(/(^|\s)ask the user to approve(?! first; commit)/i);
  // Style-preference sentences must NOT live in the runtime directive — they
  // belong in the consuming agent's memory / project CLAUDE.md. See
  // wi-d716867f589a.
  expect(text).not.toMatch(/Co-Authored-By/);
  expect(text).not.toMatch(/Conventional-Commits/);
  expect(text).not.toMatch(/self-attribution/);
  // Neutral closer points at the repo's conventions.
  expect(text).toMatch(/commit-message conventions/i);
});

test("buildAutoCommitStopReason: with a commit_point (auto-mode row) includes the id", () => {
  const text = buildAutoCommitStopReason({
    id: "cp-xyz",
    thread_id: "b1",
    sort_index: 0,
    mode: "auto",
    status: "pending",
    commit_sha: null,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    completed_at: null,
  });
  expect(text).toContain("cp-xyz");
  expect(text).toContain("commit_point_id");
  expect(text).not.toMatch(/auto: true/);
});

test("buildCommitPointStopReason: approve-mode directive keeps the user-approval gate", () => {
  const text = buildCommitPointStopReason({
    id: "cp-1",
    thread_id: "b1",
    sort_index: 0,
    mode: "approve",
    status: "pending",
    commit_sha: null,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    completed_at: null,
  });
  expect(text).toMatch(/ask the user to approve/i);
  expect(text).toContain("cp-1");
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
  const { default: threadFactory } = await Promise.resolve({ default: (overrides = {}) => ({
    id: "b1", stream_id: "s1", title: "B", status: "active" as ThreadStatus, sort_index: 0,
    pane_target: "p", resume_session_id: "", auto_commit: false, custom_prompt: null,
    created_at: "2024-01-01T00:00:00Z", updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  }) });
  const cp = {
    id: "cp1", thread_id: "b1", sort_index: 1, mode: "approve" as const,
    status: "pending" as const, commit_sha: null,
    created_at: "2024-01-01T00:00:00Z", updated_at: "2024-01-01T00:00:00Z", completed_at: null,
  };
  const out = decideStopDirective(
    { thread: threadFactory(), commitPoints: [cp], waitPoints: [], workItems: [], readyWorkItems: [] },
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
  const threadStore = new ThreadStore(dir);
  const state = threadStore.ensureStream(stream);
  const threadId = state.threads[0]!.id;
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
  return { dir, stream, streamStore, threadStore, threadId, workItems, turnStore, effortStore, snapshotStore, flushSnapshot };
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
      { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot },
      { threadId: h.threadId, workItemId: item.id, previous: "ready", next: "in_progress" },
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
      threadId: h.threadId, kind: "task", title: "T",
      createdBy: "user", actorId: "test",
    });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    applyStatusTransition(
      { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot },
      { threadId: h.threadId, workItemId: item.id, previous: "ready", next: "in_progress" },
    );
    writeFileSync(join(h.dir, "a.txt"), "v2");
    applyStatusTransition(
      { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot },
      { threadId: h.threadId, workItemId: item.id, previous: "in_progress", next: "human_check" },
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
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    writeFileSync(join(h.dir, "a.txt"), "v1");
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: item.id, previous: "ready", next: "in_progress" });
    writeFileSync(join(h.dir, "a.txt"), "v2");
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: item.id, previous: "in_progress", next: "human_check" });
    writeFileSync(join(h.dir, "a.txt"), "v3");
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: item.id, previous: "human_check", next: "ready" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: item.id, previous: "ready", next: "in_progress" });
    expect(h.effortStore.listEffortsForWorkItem(item.id)).toHaveLength(2);
    expect(h.effortStore.getOpenEffort(item.id)).not.toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("opening a turn after an effort opens attaches it via linkEffortTurn", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "T",
      createdBy: "user", actorId: "test",
    });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    applyStatusTransition(
      { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot },
      { threadId: h.threadId, workItemId: item.id, previous: "ready", next: "in_progress" },
    );
    const turn = h.turnStore.openTurn({ threadId: h.threadId, prompt: "P" });
    linkOpenEffortsToTurn(h.effortStore, turn.id);
    const effort = h.effortStore.getOpenEffort(item.id)!;
    expect(h.effortStore.listTurnsForEffort(effort.id)).toEqual([turn.id]);
    expect(h.effortStore.listEffortsForTurn(turn.id).map((e) => e.id)).toEqual([effort.id]);
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("effort opening during an already-open turn links back to that turn", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "T",
      createdBy: "user", actorId: "test",
    });
    // Turn open first...
    const turn = h.turnStore.openTurn({ threadId: h.threadId, prompt: "P" });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    // ...then the task transitions to in_progress. applyStatusTransition
    // should see the open turn via currentOpenTurn and link itself.
    applyStatusTransition(
      { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot },
      { threadId: h.threadId, workItemId: item.id, previous: "ready", next: "in_progress" },
    );
    const effort = h.effortStore.getOpenEffort(item.id)!;
    expect(h.effortStore.listTurnsForEffort(effort.id)).toEqual([turn.id]);
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("status changes with same-status or undefined-next are no-ops", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "T",
      createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
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
      ignore: (rel) => rel.startsWith(".newde") || rel.startsWith(".git"),
    });
    expect(again.created).toBe(false);
    rmSync(h.dir, { recursive: true, force: true });
  });
});

describe("touchedFiles payload on human_check transition", () => {
  test("update_work_item with touchedFiles populates work_item_effort_file on transition to human_check", () => {
    const h = seedHistoryHarness();
    const a = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "A", createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    const openEffort = h.effortStore.getOpenEffort(a.id)!;
    writeFileSync(join(h.dir, "a.txt"), "v1");
    writeFileSync(join(h.dir, "b.txt"), "v1");
    applyStatusTransition(deps, {
      threadId: h.threadId, workItemId: a.id,
      previous: "in_progress", next: "human_check",
      touchedFiles: ["a.txt", "b.txt", "a.txt"],
    });
    expect(h.effortStore.listEffortFiles(openEffort.id)).toEqual(["a.txt", "b.txt"]);
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("transitions to ready/blocked ignore touchedFiles", () => {
    const h = seedHistoryHarness();
    const a = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "A", createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    const openEffort = h.effortStore.getOpenEffort(a.id)!;
    applyStatusTransition(deps, {
      threadId: h.threadId, workItemId: a.id,
      previous: "in_progress", next: "blocked",
      touchedFiles: ["a.txt"],
    });
    expect(h.effortStore.listEffortFiles(openEffort.id)).toEqual([]);

    // Re-open, then close to ready — also ignored.
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "blocked", next: "in_progress" });
    const secondEffort = h.effortStore.getOpenEffort(a.id)!;
    applyStatusTransition(deps, {
      threadId: h.threadId, workItemId: a.id,
      previous: "in_progress", next: "ready",
      touchedFiles: ["b.txt"],
    });
    expect(h.effortStore.listEffortFiles(secondEffort.id)).toEqual([]);
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("touchedFiles with >100 paths inserts nothing (server cap)", () => {
    const h = seedHistoryHarness();
    const a = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "A", createdBy: "user", actorId: "test",
    });
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    const openEffort = h.effortStore.getOpenEffort(a.id)!;
    const many: string[] = [];
    for (let i = 0; i < 101; i++) many.push(`f${i}.txt`);
    applyStatusTransition(deps, {
      threadId: h.threadId, workItemId: a.id,
      previous: "in_progress", next: "human_check",
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
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    writeFileSync(join(h.dir, "a.txt"), "v1");
    writeFileSync(join(h.dir, "b.txt"), "v1");
    // No recordEffortFile calls — log is empty for this effort.
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "in_progress", next: "human_check" });
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
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
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
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "in_progress", next: "human_check" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: b.id, previous: "in_progress", next: "human_check" });

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
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: b.id, previous: "ready", next: "in_progress" });
    const effA = h.effortStore.getOpenEffort(a.id)!;
    const effB = h.effortStore.getOpenEffort(b.id)!;
    writeFileSync(join(h.dir, "shared.txt"), "final");
    h.effortStore.recordEffortFile(effA.id, "shared.txt");
    h.effortStore.recordEffortFile(effB.id, "shared.txt");
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "in_progress", next: "human_check" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: b.id, previous: "in_progress", next: "human_check" });
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
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: b.id, previous: "ready", next: "in_progress" });
    writeFileSync(join(h.dir, "a.txt"), "by-a");
    writeFileSync(join(h.dir, "b.txt"), "by-b");
    // Neither transition declares touchedFiles — effort_file log stays empty.
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "in_progress", next: "human_check" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: b.id, previous: "in_progress", next: "human_check" });
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
      { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot },
      { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" },
    );
    const effA = h.effortStore.getOpenEffort(a.id)!;
    expect(computeEffortFiles(h.effortStore, h.snapshotStore, effA.id)).toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });
});

describe("composeAutoCompleteNote", () => {
  test("lists all paths when 5 or fewer", () => {
    expect(composeAutoCompleteNote({ filePaths: ["a.ts", "b.ts"] }))
      .toBe("Auto-summary: touched 2 files: a.ts, b.ts.");
  });
  test("truncates to first 5 with `…and M more` when >5", () => {
    expect(composeAutoCompleteNote({ filePaths: ["a", "b", "c", "d", "e", "f", "g"] }))
      .toBe("Auto-summary: touched 7 files: a, b, c, d, e …and 2 more.");
  });
  test("handles empty list", () => {
    expect(composeAutoCompleteNote({ filePaths: [] }))
      .toBe("Auto-summary: no file changes detected.");
  });
  test("prepends test-result when present (clean run)", () => {
    expect(composeAutoCompleteNote({ filePaths: ["a.ts"], testResult: { pass: 12, fail: 0 } }))
      .toBe("Tests: 12/0. Auto-summary: touched 1 file: a.ts.");
  });
  test("prepends test-result with failing count when fail > 0", () => {
    expect(composeAutoCompleteNote({ filePaths: ["a.ts"], testResult: { pass: 12, fail: 3 } }))
      .toBe("Tests: 12/3 (3 failing). Auto-summary: touched 1 file: a.ts.");
  });
  test("prepends tsc: clean when tscErrors=0", () => {
    expect(composeAutoCompleteNote({ filePaths: ["a.ts"], tscErrors: 0 }))
      .toBe("tsc: clean. Auto-summary: touched 1 file: a.ts.");
  });
  test("prepends TS errors when tscErrors>0", () => {
    expect(composeAutoCompleteNote({ filePaths: ["a.ts"], tscErrors: 7 }))
      .toBe("TS errors: 7. Auto-summary: touched 1 file: a.ts.");
  });
  test("includes commit shas when present", () => {
    expect(composeAutoCompleteNote({ filePaths: ["a.ts"], commitShas: ["abc1234"] }))
      .toBe("commits: abc1234. Auto-summary: touched 1 file: a.ts.");
  });
  test("combines signals in order: tests, tsc, commits", () => {
    expect(composeAutoCompleteNote({
      filePaths: ["a.ts"],
      testResult: { pass: 484, fail: 0 },
      tscErrors: 0,
      commitShas: ["abc1234"],
    })).toBe("Tests: 484/0. tsc: clean. commits: abc1234. Auto-summary: touched 1 file: a.ts.");
  });
  test("clamps total to AUTO_COMPLETE_NOTE_MAX_LEN chars", () => {
    const many = Array.from({ length: 200 }, (_, i) => `very/long/path/to/file-${i}.ts`);
    const out = composeAutoCompleteNote({ filePaths: many });
    expect(out.length).toBeLessThanOrEqual(AUTO_COMPLETE_NOTE_MAX_LEN);
  });
  test("signals-absent falls back to file-list format unchanged", () => {
    expect(composeAutoCompleteNote({ filePaths: ["a.ts"] }))
      .toBe("Auto-summary: touched 1 file: a.ts.");
  });
  test("dedupes paths", () => {
    expect(composeAutoCompleteNote({ filePaths: ["a", "a", "b"] }))
      .toBe("Auto-summary: touched 2 files: a, b.");
  });
});

describe("detectTscErrorsFromBashOutput", () => {
  test("counts `error TSxxxx` occurrences", () => {
    const out = "src/foo.ts(3,5): error TS2322: Type 'string' is not assignable to type 'number'.\n"
      + "src/bar.ts(10,1): error TS2304: Cannot find name 'baz'.";
    expect(detectTscErrorsFromBashOutput(out)).toBe(2);
  });
  test("returns 0 when tsc runs clean (tsc mentioned but no errors)", () => {
    expect(detectTscErrorsFromBashOutput("$ bunx tsc --noEmit\n(exit 0)")).toBe(0);
    expect(detectTscErrorsFromBashOutput("Found 0 errors. Watching for file changes.")).toBe(0);
  });
  test("returns null when output is not a tsc run", () => {
    expect(detectTscErrorsFromBashOutput("hello world")).toBeNull();
    expect(detectTscErrorsFromBashOutput(null)).toBeNull();
    expect(detectTscErrorsFromBashOutput("")).toBeNull();
  });
});

describe("extractBashStdout", () => {
  test("returns stdout field when object has one", () => {
    expect(extractBashStdout({ stdout: "hello", stderr: "" })).toBe("hello");
  });
  test("concatenates stdout + stderr when both present", () => {
    expect(extractBashStdout({ stdout: "a", stderr: "b" })).toBe("a\nb");
  });
  test("returns null for malformed responses", () => {
    expect(extractBashStdout(null)).toBeNull();
    expect(extractBashStdout(undefined)).toBeNull();
    expect(extractBashStdout({})).toBeNull();
  });
});

describe("extractTodoList", () => {
  test("extracts todos array from TodoWrite payload", () => {
    const out = extractTodoList({
      todos: [
        { content: "step 1", status: "completed" },
        { content: "step 2", status: "in_progress" },
      ],
    });
    expect(out).toEqual([
      { content: "step 1", status: "completed" },
      { content: "step 2", status: "in_progress" },
    ]);
  });
  test("defaults missing status to pending", () => {
    const out = extractTodoList({ todos: [{ content: "x" }] });
    expect(out).toEqual([{ content: "x", status: "pending" }]);
  });
  test("returns null for malformed payload", () => {
    expect(extractTodoList(null)).toBeNull();
    expect(extractTodoList({})).toBeNull();
    expect(extractTodoList({ todos: "not array" })).toBeNull();
  });
});

describe("composeTaskListNote", () => {
  test("formats one line per step with final-status glyphs", () => {
    const out = composeTaskListNote([
      { content: "Read runtime.ts", status: "completed" },
      { content: "Write failing test", status: "completed" },
      { content: "Fix bug", status: "pending" },
    ]);
    expect(out).toBe("TaskCreate breakdown: ☑ Read runtime.ts / ☑ Write failing test / ☐ Fix bug");
  });
  test("marks in_progress with ▶", () => {
    const out = composeTaskListNote([
      { content: "a", status: "in_progress" },
    ]);
    expect(out).toContain("▶ a");
  });
  test("returns null for empty list", () => {
    expect(composeTaskListNote([])).toBeNull();
  });
  test("caps to AUTO_COMPLETE_NOTE_MAX_LEN", () => {
    const many = Array.from({ length: 50 }, (_, i) => ({ content: `step number ${i} with some words`, status: "completed" }));
    const out = composeTaskListNote(many);
    expect(out).not.toBeNull();
    expect(out!.length).toBeLessThanOrEqual(AUTO_COMPLETE_NOTE_MAX_LEN);
  });
});

describe("detectCommitShasFromBashOutput", () => {
  test("extracts sha from `[branch abc1234]` git commit line", () => {
    expect(detectCommitShasFromBashOutput("[main abc1234] fix bug"))
      .toEqual(["abc1234"]);
  });
  test("dedupes multiple matches, keeps order", () => {
    const out = "[main abc1234] first\n[main def5678] second\n[main abc1234] noop";
    expect(detectCommitShasFromBashOutput(out)).toEqual(["abc1234", "def5678"]);
  });
  test("returns null when no sha found", () => {
    expect(detectCommitShasFromBashOutput("nothing here")).toBeNull();
    expect(detectCommitShasFromBashOutput(null)).toBeNull();
  });
});

describe("detectTestResultFromBashOutput", () => {
  test("matches bun test summary line", () => {
    const output = "bun test v1.3.9\n...\n 27 pass\n 0 fail\n 63 expect() calls";
    expect(detectTestResultFromBashOutput(output)).toEqual({ pass: 27, fail: 0 });
  });
  test("matches embedded pass/fail combined on one line", () => {
    expect(detectTestResultFromBashOutput(" 5 pass  2 fail"))
      .toEqual({ pass: 5, fail: 2 });
  });
  test("returns null when no match", () => {
    expect(detectTestResultFromBashOutput("hello world")).toBeNull();
    expect(detectTestResultFromBashOutput(null)).toBeNull();
    expect(detectTestResultFromBashOutput("")).toBeNull();
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
  test("Bash with other/unknown command is write-intent (err toward auto-file)", () => {
    expect(isWriteIntentTool("Bash", { command: "npm install foo" })).toBe(true);
    expect(isWriteIntentTool("Bash", { command: "rm -rf dist" })).toBe(true);
  });
});

describe("deriveAutoItemTitleFromPrompt", () => {
  test("takes first 60 chars of prompt, trimmed", () => {
    expect(deriveAutoItemTitleFromPrompt("  fix bar in foo.ts  "))
      .toBe("fix bar in foo.ts");
  });
  test("truncates at 60 chars with ellipsis", () => {
    const long = "a".repeat(80);
    const out = deriveAutoItemTitleFromPrompt(long);
    expect(out.length).toBeLessThanOrEqual(60);
  });
  test("collapses newlines to spaces", () => {
    expect(deriveAutoItemTitleFromPrompt("line one\nline two"))
      .toBe("line one line two");
  });
  test("falls back when prompt is empty/null", () => {
    expect(deriveAutoItemTitleFromPrompt(null)).toBe("agent work");
    expect(deriveAutoItemTitleFromPrompt("")).toBe("agent work");
  });
});

describe("deriveAutoItemTitleFromDiff", () => {
  test("returns null when no files touched", () => {
    expect(deriveAutoItemTitleFromDiff([])).toBeNull();
  });
  test("single file → Edit <basename>", () => {
    expect(deriveAutoItemTitleFromDiff(["src/foo.ts"])).toBe("Edit foo.ts");
  });
  test("2-3 files → Edit <list of basenames>", () => {
    expect(deriveAutoItemTitleFromDiff(["src/a.ts", "src/b.ts"])).toBe("Edit src: a.ts, b.ts");
    expect(deriveAutoItemTitleFromDiff(["src/a.ts", "src/b.ts", "src/c.ts"])).toBe("Edit src: a.ts, b.ts, c.ts");
  });
  test("4+ files → includes +N more", () => {
    const out = deriveAutoItemTitleFromDiff(["src/a.ts", "src/b.ts", "src/c.ts", "src/d.ts", "src/e.ts"]);
    expect(out).toBe("Edit src: a.ts, b.ts, +3 more");
  });
  test("common top-level dir is prefixed", () => {
    const out = deriveAutoItemTitleFromDiff(["src/ui/App.tsx", "src/ui/api.ts", "src/ui/components/X.tsx", "src/ui/components/Y.tsx"]);
    expect(out).toBe("Edit src: App.tsx, api.ts, +2 more");
  });
  test("no common top-level dir uses bare basenames", () => {
    const out = deriveAutoItemTitleFromDiff(["src/a.ts", "other/b.ts"]);
    expect(out).toBe("Edit a.ts, b.ts");
  });
  test("output capped at 60 chars", () => {
    const files = Array.from({ length: 10 }, (_, i) => `src/extremely-long-filename-number-${i}.ts`);
    const out = deriveAutoItemTitleFromDiff(files);
    expect(out!.length).toBeLessThanOrEqual(60);
  });
});

describe("autoFileWorkItemIfNeeded", () => {
  test("creates an agent-auto in_progress item when none exists", () => {
    const h = seedHistoryHarness();
    const id = autoFileWorkItemIfNeeded(
      { workItemStore: h.workItems },
      { threadId: h.threadId, prompt: "rename foo to bar in baz.ts" },
    );
    expect(id).toBeTruthy();
    const item = h.workItems.getItem(h.threadId, id!)!;
    expect(item.author).toBe("agent-auto");
    expect(item.status).toBe("in_progress");
    expect(item.title).toBe("rename foo to bar in baz.ts");
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("no-op when an open agent-auto item already exists", () => {
    const h = seedHistoryHarness();
    const first = autoFileWorkItemIfNeeded(
      { workItemStore: h.workItems },
      { threadId: h.threadId, prompt: "first" },
    );
    const second = autoFileWorkItemIfNeeded(
      { workItemStore: h.workItems },
      { threadId: h.threadId, prompt: "second" },
    );
    expect(first).toBeTruthy();
    expect(second).toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("guard is thread-wide, not per-turn — two prompts, one auto item", () => {
    const h = seedHistoryHarness();
    // Turn A prompt → first write-intent fires auto-file.
    const a = autoFileWorkItemIfNeeded(
      { workItemStore: h.workItems },
      { threadId: h.threadId, prompt: "turn A prompt" },
    );
    expect(a).toBeTruthy();
    // Turn A "ends" (we simulate a fresh turn without closing the auto item —
    // matches subagent / cross-turn cases where Stop hasn't yet auto-completed).
    // Turn B prompt → second write-intent must NOT create a duplicate.
    const b = autoFileWorkItemIfNeeded(
      { workItemStore: h.workItems },
      { threadId: h.threadId, prompt: "turn B prompt" },
    );
    expect(b).toBeNull();
    // Sanity: only one agent-auto row exists for the thread.
    const all = h.workItems.listItems(h.threadId)
      .filter((it) => it.author === "agent-auto");
    expect(all).toHaveLength(1);
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("adoptAutoItem flips author to 'agent' and overwrites fields in place", () => {
    const h = seedHistoryHarness();
    const id = autoFileWorkItemIfNeeded(
      { workItemStore: h.workItems },
      { threadId: h.threadId, prompt: "initial prompt" },
    )!;
    const adopted = h.workItems.adoptAutoItem({
      threadId: h.threadId,
      itemId: id,
      title: "new title",
      description: "new desc",
      kind: "task",
      actorKind: "agent",
      actorId: "mcp",
    });
    expect(adopted.id).toBe(id);
    expect(adopted.author).toBe("agent");
    expect(adopted.title).toBe("new title");
    // The finder should no longer return it — it's been adopted.
    expect(h.workItems.findOpenAutoItemForThread(h.threadId)).toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });
});

describe("autoCompleteOpenAutoItems", () => {
  test("flips the open agent-auto item to human_check and adds an auto-summary note", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "auto row",
      status: "in_progress", createdBy: "agent", actorId: "runtime",
      author: "agent-auto",
    });
    // Open effort + link to a turn so the turn-effort check passes.
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    // A fresh opening effort to match the in_progress state.
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: item.id, previous: "ready", next: "in_progress" });
    const turn = h.turnStore.openTurn({ threadId: h.threadId, prompt: "fix foo" });
    const eff = h.effortStore.getOpenEffort(item.id)!;
    h.effortStore.linkEffortTurn(eff.id, turn.id);

    const closedId = autoCompleteOpenAutoItems(
      { workItemStore: h.workItems, effortStore: h.effortStore },
      { threadId: h.threadId, turnId: turn.id, filePaths: ["a.ts", "b.ts"] },
    );
    expect(closedId).toBe(item.id);
    const after = h.workItems.getItem(h.threadId, item.id)!;
    expect(after.status).toBe("human_check");
    const notes = h.workItems.getWorkNotes(item.id);
    expect(notes).toHaveLength(1);
    expect(notes[0]!.body).toBe("Auto-summary: touched 2 files: a.ts, b.ts.");
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("rewrites prompt-derived title using diff paths", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      threadId: h.threadId, kind: "task",
      title: "please look at the failing test and fix it thanks",
      status: "in_progress", createdBy: "agent", actorId: "runtime",
      author: "agent-auto",
    });
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: item.id, previous: "ready", next: "in_progress" });
    const turn = h.turnStore.openTurn({ threadId: h.threadId, prompt: "please..." });
    const eff = h.effortStore.getOpenEffort(item.id)!;
    h.effortStore.linkEffortTurn(eff.id, turn.id);
    autoCompleteOpenAutoItems(
      { workItemStore: h.workItems, effortStore: h.effortStore },
      { threadId: h.threadId, turnId: turn.id, filePaths: ["src/foo.ts", "src/bar.ts", "src/baz.ts"] },
    );
    const after = h.workItems.getItem(h.threadId, item.id)!;
    expect(after.title).not.toBe("please look at the failing test and fix it thanks");
    expect(after.title.length).toBeLessThanOrEqual(60);
    expect(after.title).toContain("foo.ts");
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("no-op when no agent-auto item exists", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "manual",
      status: "in_progress", createdBy: "user", actorId: "ui",
    });
    const turn = h.turnStore.openTurn({ threadId: h.threadId, prompt: "x" });
    expect(autoCompleteOpenAutoItems(
      { workItemStore: h.workItems, effortStore: h.effortStore },
      { threadId: h.threadId, turnId: turn.id, filePaths: ["a.ts"] },
    )).toBeNull();
    const after = h.workItems.getItem(h.threadId, item.id)!;
    expect(after.status).toBe("in_progress");
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("no-op when the auto item has no effort linked to the current turn", () => {
    const h = seedHistoryHarness();
    const item = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "old auto row",
      status: "in_progress", createdBy: "agent", actorId: "runtime",
      author: "agent-auto",
    });
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: item.id, previous: "ready", next: "in_progress" });
    // Intentionally open a turn but DO NOT link the effort to it.
    const turn = h.turnStore.openTurn({ threadId: h.threadId, prompt: "x" });
    expect(autoCompleteOpenAutoItems(
      { workItemStore: h.workItems, effortStore: h.effortStore },
      { threadId: h.threadId, turnId: turn.id, filePaths: ["a.ts"] },
    )).toBeNull();
    rmSync(h.dir, { recursive: true, force: true });
  });

  // --- wi-4daabc5e1dae: orchestrator dispatch-only turns close their auto-item ---

  test("pure-dispatch turn (no file changes, no effort) closes the auto-item with a coordination note and discovered_from links", () => {
    const h = seedHistoryHarness();
    // Dispatched children — imagine the orchestrator created + dispatched these.
    const childA = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "child A",
      status: "in_progress", createdBy: "agent", actorId: "runtime",
    });
    const childB = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "child B",
      status: "in_progress", createdBy: "agent", actorId: "runtime",
    });
    // The orchestrator's own auto-filed item — no effort, no file changes.
    const auto = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "do it all",
      status: "in_progress", createdBy: "agent", actorId: "runtime-auto",
      author: "agent-auto",
    });
    const turn = h.turnStore.openTurn({ threadId: h.threadId, prompt: "do it all" });

    const closedId = autoCompleteOpenAutoItems(
      { workItemStore: h.workItems, effortStore: h.effortStore },
      {
        threadId: h.threadId,
        turnId: turn.id,
        filePaths: [],
        dispatchCount: 2,
        dispatchedItemIds: [childA.id, childB.id],
      },
    );
    expect(closedId).toBe(auto.id);
    const after = h.workItems.getItem(h.threadId, auto.id)!;
    expect(after.status).toBe("human_check");
    const notes = h.workItems.getWorkNotes(auto.id);
    expect(notes).toHaveLength(1);
    expect(notes[0]!.body).toMatch(/Coordinated 2 subagent dispatch/);
    // discovered_from links: child.id -> auto.id (the children were uncovered
    // while working on the orchestrator auto-item).
    const autoDetail = h.workItems.getItemDetail(h.threadId, auto.id)!;
    const fromIds = autoDetail.incoming
      .filter((l) => l.link_type === "discovered_from")
      .map((l) => l.from_item_id)
      .sort();
    expect(fromIds).toEqual([childA.id, childB.id].sort());
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("pure-dispatch: no dispatchCount and no effort → still a no-op (no auto close)", () => {
    const h = seedHistoryHarness();
    const auto = h.workItems.createItem({
      threadId: h.threadId, kind: "task", title: "stuck",
      status: "in_progress", createdBy: "agent", actorId: "runtime-auto",
      author: "agent-auto",
    });
    const turn = h.turnStore.openTurn({ threadId: h.threadId, prompt: "x" });
    expect(autoCompleteOpenAutoItems(
      { workItemStore: h.workItems, effortStore: h.effortStore },
      { threadId: h.threadId, turnId: turn.id, filePaths: [], dispatchCount: 0 },
    )).toBeNull();
    const after = h.workItems.getItem(h.threadId, auto.id)!;
    expect(after.status).toBe("in_progress");
    rmSync(h.dir, { recursive: true, force: true });
  });
});

describe("linkCommitToContributingItems", () => {
  test("inserts one work_item_commit row per item closed since latest done commit", () => {
    const h = seedHistoryHarness();
    const commitJunction = new WorkItemCommitStore(h.dir);
    const a = h.workItems.createItem({ threadId: h.threadId, kind: "task", title: "A", status: "in_progress", createdBy: "user", actorId: "t" });
    const b = h.workItems.createItem({ threadId: h.threadId, kind: "task", title: "B", status: "in_progress", createdBy: "user", actorId: "t" });
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    // Open + close efforts so `listClosedEffortsForThreadAfter` sees both.
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "in_progress", next: "human_check" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: b.id, previous: "ready", next: "in_progress" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: b.id, previous: "in_progress", next: "human_check" });

    const inserted = linkCommitToContributingItems(
      { effortStore: h.effortStore, workItemCommitStore: commitJunction },
      { threadId: h.threadId, sha: "sha-xyz", latestDoneCompletedAt: null },
    );
    expect(inserted.sort()).toEqual([a.id, b.id].sort());
    const rows = commitJunction.listItemsForSha("sha-xyz").map((r) => r.work_item_id).sort();
    expect(rows).toEqual([a.id, b.id].sort());
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("filters efforts by latestDoneCompletedAt cutoff", () => {
    const h = seedHistoryHarness();
    const commitJunction = new WorkItemCommitStore(h.dir);
    const a = h.workItems.createItem({ threadId: h.threadId, kind: "task", title: "A", status: "in_progress", createdBy: "user", actorId: "t" });
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "in_progress", next: "human_check" });

    // Future cutoff → no items qualify.
    const futureIso = "2099-01-01T00:00:00Z";
    const inserted = linkCommitToContributingItems(
      { effortStore: h.effortStore, workItemCommitStore: commitJunction },
      { threadId: h.threadId, sha: "sha-nope", latestDoneCompletedAt: futureIso },
    );
    expect(inserted).toEqual([]);
    expect(commitJunction.listItemsForSha("sha-nope")).toEqual([]);
    rmSync(h.dir, { recursive: true, force: true });
  });
});

describe("backfillCommitLinksForThread (wi-ec4c8e6f44fd)", () => {
  test("populates junction rows for settled items when an ad-hoc sha arrives", () => {
    const h = seedHistoryHarness();
    const commitJunction = new WorkItemCommitStore(h.dir);
    const commitPointStore = new CommitPointStore(h.dir);
    const a = h.workItems.createItem({ threadId: h.threadId, kind: "task", title: "A", status: "in_progress", createdBy: "user", actorId: "t" });
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "in_progress", next: "human_check" });

    const inserted = backfillCommitLinksForThread(
      { effortStore: h.effortStore, workItemCommitStore: commitJunction, commitPointStore },
      { threadId: h.threadId, sha: "sha-adhoc" },
    );
    expect(inserted).toEqual([a.id]);
    const rows = commitJunction.listItemsForSha("sha-adhoc").map((r) => r.work_item_id);
    expect(rows).toEqual([a.id]);
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("sha that is already linked → no-op", () => {
    const h = seedHistoryHarness();
    const commitJunction = new WorkItemCommitStore(h.dir);
    const commitPointStore = new CommitPointStore(h.dir);
    const a = h.workItems.createItem({ threadId: h.threadId, kind: "task", title: "A", status: "in_progress", createdBy: "user", actorId: "t" });
    const deps = { effortStore: h.effortStore, turnStore: h.turnStore, flushSnapshot: h.flushSnapshot };
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "ready", next: "in_progress" });
    applyStatusTransition(deps, { threadId: h.threadId, workItemId: a.id, previous: "in_progress", next: "human_check" });
    commitJunction.insert(a.id, "sha-known", new Date().toISOString());
    const inserted = backfillCommitLinksForThread(
      { effortStore: h.effortStore, workItemCommitStore: commitJunction, commitPointStore },
      { threadId: h.threadId, sha: "sha-known" },
    );
    expect(inserted).toEqual([]);
    rmSync(h.dir, { recursive: true, force: true });
  });

  test("null sha → no-op", () => {
    const h = seedHistoryHarness();
    const commitJunction = new WorkItemCommitStore(h.dir);
    const commitPointStore = new CommitPointStore(h.dir);
    const inserted = backfillCommitLinksForThread(
      { effortStore: h.effortStore, workItemCommitStore: commitJunction, commitPointStore },
      { threadId: h.threadId, sha: null },
    );
    expect(inserted).toEqual([]);
    rmSync(h.dir, { recursive: true, force: true });
  });
});

describe("isDispatchLikeTool / extractDispatchedItemIds", () => {
  test("Task, mcp__newde__dispatch_work_item, mcp__newde__file_epic_with_children are dispatch-shaped", () => {
    expect(isDispatchLikeTool("Task")).toBe(true);
    expect(isDispatchLikeTool("mcp__newde__dispatch_work_item")).toBe(true);
    expect(isDispatchLikeTool("mcp__newde__file_epic_with_children")).toBe(true);
  });

  test("Edit / Bash / TodoWrite are NOT dispatch-shaped", () => {
    expect(isDispatchLikeTool("Edit")).toBe(false);
    expect(isDispatchLikeTool("Bash")).toBe(false);
    expect(isDispatchLikeTool("TodoWrite")).toBe(false);
  });

  test("extracts itemId field from MCP dispatch_work_item input", () => {
    const ids = extractDispatchedItemIds("mcp__newde__dispatch_work_item", { threadId: "b1", itemId: "wi-abc123" });
    expect(ids).toEqual(["wi-abc123"]);
  });

  test("scans the Task prompt text for wi-<hex> tokens", () => {
    const brief = "Run wi-deadbeef and also wi-1234abcd in parallel with wi-deadbeef again";
    const ids = extractDispatchedItemIds("Task", { prompt: brief });
    expect(ids.sort()).toEqual(["wi-1234abcd", "wi-deadbeef"]);
  });

  test("ignores non-object input", () => {
    expect(extractDispatchedItemIds("Task", null)).toEqual([]);
    expect(extractDispatchedItemIds("Task", "just text")).toEqual([]);
  });
});
