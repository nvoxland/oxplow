import { expect, test } from "bun:test";
import { buildAgentCommand } from "./daemon.js";
import type { Stream } from "./stream-store.js";

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
  const command = buildAgentCommand("claude", makeStream(), "working", "/tmp/session settings.json");
  expect(command).toContain("/tmp/stream one");
  expect(command).toContain("resume-working");
  expect(command).toContain("/tmp/session settings.json");
  expect(command).toContain("cd ");
  expect(command).toContain("exec claude");
  expect(command).toContain("[newde] saved resume id was stale; starting a fresh Claude session");
});

test("buildAgentCommand can append a batch-specific system prompt for Claude", () => {
  const command = buildAgentCommand(
    "claude",
    makeStream(),
    "working",
    "/tmp/session settings.json",
    "You are working in batch b-123. Always pass batchId: b-123.",
  );
  expect(command).toContain("--append-system-prompt");
  expect(command).toContain("batch b-123");
});

test("buildAgentCommand can include session-specific mcp config for Claude", () => {
  const command = buildAgentCommand(
    "claude",
    makeStream(),
    "working",
    "/tmp/settings.json",
    undefined,
    "{\"mcpServers\":{\"newde\":{\"type\":\"stdio\",\"command\":\"node\",\"args\":[\"/tmp/mcp.cjs\"]}}}",
  );
  expect(command).toContain("--mcp-config");
  expect(command).toContain("--strict-mcp-config");
  expect(command).toContain("/tmp/mcp.cjs");
});

test("buildAgentCommand uses the pane-specific resume id", () => {
  const command = buildAgentCommand("claude", makeStream(), "talking", "/tmp/settings.json");
  expect(command).toContain("resume-talking");
  expect(command).not.toContain("resume-working");
});

test("buildAgentCommand omits resume when none is saved", () => {
  const command = buildAgentCommand(
    "claude",
    makeStream({
      resume: {
        working_session_id: "",
        talking_session_id: "",
      },
    }),
    "working",
    "/tmp/settings.json",
  );
  expect(command).toContain("exec claude --settings");
  expect(command).toContain("/tmp/settings.json");
  expect(command).not.toContain("--resume");
});

test("buildAgentCommand launches Copilot from the stream worktree", () => {
  const command = buildAgentCommand("copilot", makeStream(), "working");
  expect(command).toContain("/tmp/stream one");
  expect(command).toContain("exec copilot");
  expect(command).not.toContain("--resume");
  expect(command).not.toContain("--settings");
});
