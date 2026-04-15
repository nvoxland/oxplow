import { expect, test } from "bun:test";
import { buildClaudeCommand } from "./daemon.js";
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

test("buildClaudeCommand launches Claude from the stream worktree", () => {
  const command = buildClaudeCommand(makeStream(), "working", "/tmp/session settings.json");
  expect(command).toContain("/tmp/stream one");
  expect(command).toContain("resume-working");
  expect(command).toContain("/tmp/session settings.json");
  expect(command).toContain("cd ");
  expect(command).toContain("exec claude");
  expect(command).toContain("[newde] saved resume id was stale; starting a fresh Claude session");
});

test("buildClaudeCommand uses the pane-specific resume id", () => {
  const command = buildClaudeCommand(makeStream(), "talking", "/tmp/settings.json");
  expect(command).toContain("resume-talking");
  expect(command).not.toContain("resume-working");
});

test("buildClaudeCommand omits resume when none is saved", () => {
  const command = buildClaudeCommand(
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
