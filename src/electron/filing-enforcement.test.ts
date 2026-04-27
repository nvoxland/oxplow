import { describe, expect, test } from "bun:test";
import type { Thread } from "../persistence/thread-store.js";
import { buildFilingEnforcementPreToolDeny } from "./filing-enforcement.js";

function thread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: "b-test",
    stream_id: "s-test",
    title: "Default",
    status: "active",
    created_at: 0,
    updated_at: 0,
    sort_index: 0,
    resume_session_id: null,
    last_session_id: null,
    last_prompt: null,
    last_prompt_at: null,
    paused_at: null,
    rest_color: null,
    rest_label: null,
    ...overrides,
  } as Thread;
}

describe("buildFilingEnforcementPreToolDeny", () => {
  test("blocks Edit on writer thread with no in_progress item and no filing this turn", () => {
    const out = buildFilingEnforcementPreToolDeny({
      thread: thread(),
      toolName: "Edit",
      hasInProgressItem: false,
      filedThisTurn: false,
    });
    expect(out).not.toBeNull();
    expect(out?.hookSpecificOutput.permissionDecision).toBe("deny");
    expect(out?.hookSpecificOutput.permissionDecisionReason).toContain("Edit");
    expect(out?.hookSpecificOutput.permissionDecisionReason).toContain("create_work_item");
  });

  test("blocks Write, MultiEdit, NotebookEdit identically", () => {
    for (const toolName of ["Write", "MultiEdit", "NotebookEdit"]) {
      const out = buildFilingEnforcementPreToolDeny({
        thread: thread(),
        toolName,
        hasInProgressItem: false,
        filedThisTurn: false,
      });
      expect(out).not.toBeNull();
      expect(out?.hookSpecificOutput.permissionDecisionReason).toContain(toolName);
    }
  });

  test("allows when an in_progress item exists", () => {
    const out = buildFilingEnforcementPreToolDeny({
      thread: thread(),
      toolName: "Edit",
      hasInProgressItem: true,
      filedThisTurn: false,
    });
    expect(out).toBeNull();
  });

  test("allows when filing happened earlier this turn", () => {
    const out = buildFilingEnforcementPreToolDeny({
      thread: thread(),
      toolName: "Edit",
      hasInProgressItem: false,
      filedThisTurn: true,
    });
    expect(out).toBeNull();
  });

  test("does not enforce on Bash (git merge / codegen / etc.)", () => {
    const out = buildFilingEnforcementPreToolDeny({
      thread: thread(),
      toolName: "Bash",
      hasInProgressItem: false,
      filedThisTurn: false,
    });
    expect(out).toBeNull();
  });

  test("does not enforce on MCP / Read / Grep tools", () => {
    for (const toolName of ["mcp__oxplow__list_notes", "Read", "Grep", "Glob", "Task"]) {
      const out = buildFilingEnforcementPreToolDeny({
        thread: thread(),
        toolName,
        hasInProgressItem: false,
        filedThisTurn: false,
      });
      expect(out).toBeNull();
    }
  });

  test("does not enforce on non-writer threads (write-guard handles them)", () => {
    const out = buildFilingEnforcementPreToolDeny({
      thread: thread({ status: "queued" }),
      toolName: "Edit",
      hasInProgressItem: false,
      filedThisTurn: false,
    });
    expect(out).toBeNull();
  });

  test("returns null when thread is missing", () => {
    const out = buildFilingEnforcementPreToolDeny({
      thread: null,
      toolName: "Edit",
      hasInProgressItem: false,
      filedThisTurn: false,
    });
    expect(out).toBeNull();
  });
});
