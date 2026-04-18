import { expect, test } from "bun:test";
import { buildAgentCommand, buildAgentCommandForSession } from "./agent-command.js";
import type { Stream } from "../persistence/stream-store.js";

function makeStream(overrides: Partial<Stream> = {}): Stream {
  return {
    id: "s-1",
    title: "Stream 1",
    summary: "",
    branch: "stream-1",
    branch_ref: "refs/heads/stream-1",
    branch_source: "local",
    worktree_path: "/tmp/stream one",
    created_at: "2026-01-01T00:00:00.000Z",
    updated_at: "2026-01-01T00:00:00.000Z",
    panes: {
      working: "newde-proj:working-s-1",
      talking: "newde-proj:talking-s-1",
    },
    resume: {
      working_session_id: "resume-working",
      talking_session_id: "resume-talking",
    },
    ...overrides,
  };
}

test("buildAgentCommand launches Claude from the stream worktree", () => {
  const command = buildAgentCommand("claude", makeStream(), "working");
  expect(command).toContain("/tmp/stream one");
  expect(command).toContain("resume-working");
  expect(command).toContain("cd ");
  expect(command).toContain("exec claude");
  expect(command).toContain("[newde] saved resume id was stale; starting a fresh Claude session");
});

test("buildAgentCommand never emits --settings (no daemon path anymore)", () => {
  const command = buildAgentCommand("claude", makeStream(), "working");
  expect(command).not.toContain("--settings");
});

test("buildAgentCommand can append a batch-specific system prompt for Claude", () => {
  const command = buildAgentCommand("claude", makeStream(), "working", {
    appendSystemPrompt: "You are working in batch b-123. Always pass batchId: b-123.",
  });
  expect(command).toContain("--append-system-prompt");
  expect(command).toContain("batch b-123");
});

test("buildAgentCommand can include session-specific mcp config for Claude", () => {
  const command = buildAgentCommand("claude", makeStream(), "working", {
    mcpConfig: '{"mcpServers":{"newde":{"type":"stdio","command":"node","args":["/tmp/mcp.cjs"]}}}',
  });
  expect(command).toContain("--mcp-config");
  expect(command).toContain("--strict-mcp-config");
  expect(command).toContain("/tmp/mcp.cjs");
});

test("buildAgentCommand appends --plugin-dir when pluginDir is set", () => {
  const command = buildAgentCommand("claude", makeStream(), "working", {
    pluginDir: "/abs/.newde/runtime/claude-plugin",
  });
  expect(command).toContain("--plugin-dir");
  expect(command).toContain("/abs/.newde/runtime/claude-plugin");
});

test("buildAgentCommand appends --allowedTools with each pattern quoted", () => {
  const command = buildAgentCommand("claude", makeStream(), "working", {
    allowedTools: ["mcp__newde__*", "Read"],
  });
  expect(command).toContain("--allowedTools");
  expect(command).toContain("mcp__newde__*");
  expect(command).toContain("Read");
});

test("buildAgentCommand injects env vars before exec claude", () => {
  const command = buildAgentCommand("claude", makeStream(), "working", {
    env: { NEWDE_STREAM_ID: "s-1", NEWDE_BATCH_ID: "b-42" },
  });
  expect(command).toContain("NEWDE_STREAM_ID=");
  expect(command).toContain("NEWDE_BATCH_ID=");
  // The env assignment must appear before `exec claude` so the env flows
  // into the claude process.
  const execIdx = command.indexOf("exec claude");
  const envIdx = command.indexOf("NEWDE_BATCH_ID");
  expect(envIdx).toBeGreaterThan(0);
  expect(envIdx).toBeLessThan(execIdx);
});

test("buildAgentCommand uses the pane-specific resume id", () => {
  const command = buildAgentCommand("claude", makeStream(), "talking");
  expect(command).toContain("resume-talking");
  expect(command).not.toContain("resume-working");
});

test("buildAgentCommand omits --resume when none is saved", () => {
  const command = buildAgentCommand("claude", makeStream({
    resume: { working_session_id: "", talking_session_id: "" },
  }), "working");
  expect(command).toContain("exec claude");
  expect(command).not.toContain("--resume");
});

test("buildAgentCommandForSession with empty resumeSessionId is identical regardless of which session was previously saved", () => {
  const opts = { pluginDir: "/abs/plugin", allowedTools: ["mcp__newde__*"], appendSystemPrompt: "batch b-42" };
  const withA = buildAgentCommandForSession("claude", "/tmp/wt", "session-a", opts);
  const withB = buildAgentCommandForSession("claude", "/tmp/wt", "session-b", opts);
  expect(withA).not.toBe(withB);
  const stripped = buildAgentCommandForSession("claude", "/tmp/wt", "", opts);
  // The stripped form is a stable "launcher identity" for the batch: same
  // agent kind, worktree, and options, independent of the mutating --resume id.
  expect(stripped).not.toContain("--resume");
  expect(buildAgentCommandForSession("claude", "/tmp/wt", "", opts)).toBe(stripped);
});

test("buildAgentCommand launches Copilot from the stream worktree", () => {
  const command = buildAgentCommand("copilot", makeStream(), "working");
  expect(command).toContain("/tmp/stream one");
  expect(command).toContain("exec copilot");
  expect(command).not.toContain("--resume");
  expect(command).not.toContain("--settings");
});
