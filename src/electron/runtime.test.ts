import { expect, test } from "bun:test";
import { buildBatchMcpConfig, buildNextWorkItemStopReason, buildSessionContextBlock, describeHookHealth } from "./runtime.js";
import type { McpServerHandle } from "../mcp/mcp-server.js";

function fakeMcp(overrides: Partial<McpServerHandle> = {}): McpServerHandle {
  return {
    port: 43123,
    authToken: "secret-token",
    httpUrl: "http://127.0.0.1:43123/mcp",
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
  // Attribution warning must be present and come before human_check mention.
  expect(text).toContain("before touching any files");
  expect(text).toContain("File-change attribution");
});
