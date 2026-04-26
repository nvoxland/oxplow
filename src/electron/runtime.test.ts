import { describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { applyStatusTransition, buildAutoCommitMessage, buildAutoCommitStopReason, buildCommitPointStopReason, buildThreadMcpConfig, buildNextWorkItemStopReason, buildRecentHumanCheckReminder, buildSessionContextBlock, computeEffortFiles, describeHookHealth, isInsideWorktree, isWriteIntentTool, shouldAcceptHookFilePath, terminalInputIsInterrupt } from "./runtime.js";
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

function makeHumanCheckItem(overrides: Partial<WorkItem>): WorkItem {
  return {
    ...workItem("wi-x", "human_check" as WorkItemStatus, "Some task"),
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


test("buildNextWorkItemStopReason omits the nudge banner when uiChangeNudge is false / absent", () => {
  const text = buildNextWorkItemStopReason(
    { id: "wi-x", title: "do", kind: "task", thread_id: "b-y" },
    "s-z",
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
  // merged oxplow-runtime skill — the directive just points at it to stay terse.
  expect(text).toContain("oxplow-runtime");
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

test("buildRecentHumanCheckReminder: points at a recent agent-authored human_check item with the reopen instructions", () => {
  const now = Date.parse("2024-05-01T12:00:00Z");
  const recent = makeHumanCheckItem({
    id: "wi-abc",
    title: "Wire up the paste handler",
    updated_at: "2024-05-01T11:55:00Z", // 5 min ago
  });
  const out = buildRecentHumanCheckReminder([recent], now);
  expect(out).toContain("<recent-human-check-reminder>");
  expect(out).toContain("wi-abc");
  expect(out).toContain("Wire up the paste handler");
  expect(out).toContain("update_work_item");
  expect(out).toContain("in_progress");
  expect(out).toContain("Do NOT file a new");
});

test("buildRecentHumanCheckReminder: ignores user-authored human_check items", () => {
  const now = Date.parse("2024-05-01T12:00:00Z");
  const userItem = makeHumanCheckItem({
    id: "wi-user",
    author: "user",
    updated_at: "2024-05-01T11:55:00Z",
  });
  expect(buildRecentHumanCheckReminder([userItem], now)).toBe("");
});

test("buildRecentHumanCheckReminder: ignores items older than the window", () => {
  const now = Date.parse("2024-05-01T12:00:00Z");
  const stale = makeHumanCheckItem({
    id: "wi-old",
    updated_at: "2024-05-01T10:00:00Z", // 2h ago, outside 15-min window
  });
  expect(buildRecentHumanCheckReminder([stale], now)).toBe("");
});

test("buildRecentHumanCheckReminder: ignores items not in human_check", () => {
  const now = Date.parse("2024-05-01T12:00:00Z");
  const inProgress = makeHumanCheckItem({
    id: "wi-ip",
    status: "in_progress" as WorkItemStatus,
    updated_at: "2024-05-01T11:58:00Z",
  });
  expect(buildRecentHumanCheckReminder([inProgress], now)).toBe("");
});

test("buildRecentHumanCheckReminder: picks the most recent when multiple eligible items exist", () => {
  const now = Date.parse("2024-05-01T12:00:00Z");
  const older = makeHumanCheckItem({ id: "wi-older", updated_at: "2024-05-01T11:50:00Z" });
  const newer = makeHumanCheckItem({ id: "wi-newer", updated_at: "2024-05-01T11:58:00Z" });
  const out = buildRecentHumanCheckReminder([older, newer], now);
  expect(out).toContain("wi-newer");
  expect(out).not.toContain("wi-older");
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

  test("in_progress → human_check closes the effort with an end_snapshot_id", () => {
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
    const deps = { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot };
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

describe("touchedFiles payload on human_check transition", () => {
  test("update_work_item with touchedFiles populates work_item_effort_file on transition to human_check", () => {
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
      previous: "in_progress", next: "human_check",
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
    const deps = { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot };
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
    const deps = { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot };
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
    const deps = { effortStore: h.effortStore, flushSnapshot: h.flushSnapshot };
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
  test("Bash with other/unknown command is write-intent (err toward auto-file)", () => {
    expect(isWriteIntentTool("Bash", { command: "npm install foo" })).toBe(true);
    expect(isWriteIntentTool("Bash", { command: "rm -rf dist" })).toBe(true);
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
