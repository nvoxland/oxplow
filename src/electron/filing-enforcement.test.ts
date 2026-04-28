import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Thread } from "../persistence/thread-store.js";
import { buildFilingEnforcementPreToolDeny, isPlanModePlanFile } from "./filing-enforcement.js";

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

  describe("plan-mode plan-file exemption", () => {
    const originalHome = process.env.HOME;
    beforeEach(() => {
      process.env.HOME = "/Users/agent";
    });
    afterEach(() => {
      process.env.HOME = originalHome;
    });

    test("allows Write to ~/.claude/plans/<slug>.md with no in_progress item", () => {
      const out = buildFilingEnforcementPreToolDeny({
        thread: thread(),
        toolName: "Write",
        hasInProgressItem: false,
        filedThisTurn: false,
        filePath: "/Users/agent/.claude/plans/some-plan.md",
      });
      expect(out).toBeNull();
    });

    test("still denies a non-plan write under no-item conditions", () => {
      const out = buildFilingEnforcementPreToolDeny({
        thread: thread(),
        toolName: "Write",
        hasInProgressItem: false,
        filedThisTurn: false,
        filePath: "/Users/agent/project/src/foo.ts",
      });
      expect(out).not.toBeNull();
    });

    test("isPlanModePlanFile carve-out is narrow: requires .md extension under .claude/plans/", () => {
      expect(isPlanModePlanFile("/Users/agent/.claude/plans/p.md")).toBe(true);
      expect(isPlanModePlanFile("/Users/agent/.claude/plans/sub/p.md")).toBe(true);
      // Sibling .claude paths must NOT be exempt — only the plans dir.
      expect(isPlanModePlanFile("/Users/agent/.claude/settings.json")).toBe(false);
      expect(isPlanModePlanFile("/Users/agent/.claude/plans/p.txt")).toBe(false);
      expect(isPlanModePlanFile("/Users/agent/project/.claude/plans/p.md")).toBe(false);
      expect(isPlanModePlanFile(null)).toBe(false);
      expect(isPlanModePlanFile(undefined)).toBe(false);
      expect(isPlanModePlanFile("")).toBe(false);
    });
  });
});
