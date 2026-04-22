import { describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import type { Thread } from "../persistence/thread-store.js";
import { buildWriteGuardResponse } from "./write-guard.js";

function makeThread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: "b1",
    stream_id: "s1",
    title: "Thread 1",
    status: "queued",
    sort_index: 0,
    pane_target: "newde:0",
    resume_session_id: "",
    auto_commit: false,
    custom_prompt: null,
    created_at: "2026-04-19T00:00:00Z",
    updated_at: "2026-04-19T00:00:00Z",
    ...overrides,
  };
}

describe("buildWriteGuardResponse", () => {
  test("denies Write on a non-writer thread", () => {
    const result = buildWriteGuardResponse(makeThread({ status: "queued" }), "Write");
    expect(result?.hookSpecificOutput.permissionDecision).toBe("deny");
    expect(result?.hookSpecificOutput.hookEventName).toBe("PreToolUse");
    expect(result?.hookSpecificOutput.permissionDecisionReason).toContain("read-only");
  });

  test("denies Edit / MultiEdit / NotebookEdit on a non-writer thread", () => {
    for (const tool of ["Edit", "MultiEdit", "NotebookEdit"]) {
      expect(buildWriteGuardResponse(makeThread({ status: "queued" }), tool)).not.toBeNull();
    }
  });

  test("allows Bash on a non-writer thread (prompt-gated only)", () => {
    expect(buildWriteGuardResponse(makeThread({ status: "queued" }), "Bash")).toBeNull();
  });

  test("allows Write on the writer thread", () => {
    expect(buildWriteGuardResponse(makeThread({ status: "active" }), "Write")).toBeNull();
  });

  test("allows MCP tools from any thread", () => {
    expect(buildWriteGuardResponse(makeThread({ status: "queued" }), "mcp__newde__add_work_note")).toBeNull();
    expect(buildWriteGuardResponse(makeThread({ status: "queued" }), "mcp__newde__create_work_item")).toBeNull();
  });

  test("allows read-only tools (Read, Grep, Glob)", () => {
    for (const tool of ["Read", "Grep", "Glob"]) {
      expect(buildWriteGuardResponse(makeThread({ status: "queued" }), tool)).toBeNull();
    }
  });

  test("returns null when thread is not found", () => {
    expect(buildWriteGuardResponse(null, "Write")).toBeNull();
  });

  test("allows Write to an agent-private path outside the project root", () => {
    const parent = mkdtempSync(join(tmpdir(), "newde-wg-"));
    try {
      const projectDir = join(parent, "project");
      const outside = join(parent, "elsewhere", "plans", "foo.md");
      const result = buildWriteGuardResponse(
        makeThread({ status: "queued" }),
        "Write",
        { projectDir, toolInput: { file_path: outside } },
      );
      expect(result).toBeNull();
    } finally {
      rmSync(parent, { recursive: true, force: true });
    }
  });

  test("denies Write to an in-tree path and names the resolved path in the reason", () => {
    const parent = mkdtempSync(join(tmpdir(), "newde-wg-"));
    try {
      const projectDir = join(parent, "project");
      const inside = join(projectDir, "src", "index.ts");
      const result = buildWriteGuardResponse(
        makeThread({ status: "queued" }),
        "Write",
        { projectDir, toolInput: { file_path: inside } },
      );
      expect(result?.hookSpecificOutput.permissionDecision).toBe("deny");
      const reason = result?.hookSpecificOutput.permissionDecisionReason ?? "";
      expect(reason).toContain(resolve(inside));
      expect(reason).toContain("inside the shared worktree");
      expect(reason).toContain("read-only");
    } finally {
      rmSync(parent, { recursive: true, force: true });
    }
  });

  test("denies Write to project's .newde/ directory even from agent-private-looking path", () => {
    const parent = mkdtempSync(join(tmpdir(), "newde-wg-"));
    try {
      const projectDir = join(parent, "project");
      const newdePath = join(projectDir, ".newde", "shared-state.json");
      const result = buildWriteGuardResponse(
        makeThread({ status: "queued" }),
        "Write",
        { projectDir, toolInput: { file_path: newdePath } },
      );
      expect(result?.hookSpecificOutput.permissionDecision).toBe("deny");
    } finally {
      rmSync(parent, { recursive: true, force: true });
    }
  });

  test("relative file_path resolves against projectDir (in-tree → deny)", () => {
    const parent = mkdtempSync(join(tmpdir(), "newde-wg-"));
    try {
      const projectDir = join(parent, "project");
      const result = buildWriteGuardResponse(
        makeThread({ status: "queued" }),
        "Edit",
        { projectDir, toolInput: { file_path: "src/app.ts" } },
      );
      expect(result?.hookSpecificOutput.permissionDecision).toBe("deny");
    } finally {
      rmSync(parent, { recursive: true, force: true });
    }
  });

  test("flipping thread.status from queued to active flips the decision", () => {
    // Simulates the runtime reading thread fresh on each PreToolUse call —
    // after a promotion, the next call sees the new status and allows.
    const queued = makeThread({ status: "queued" });
    expect(buildWriteGuardResponse(queued, "Write")).not.toBeNull();

    const promoted = { ...queued, status: "active" as const };
    expect(buildWriteGuardResponse(promoted, "Write")).toBeNull();
  });
});
