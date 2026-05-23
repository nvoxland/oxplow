import { describe, expect, test } from "bun:test";

import { awaitGitOp, gitOpErrorMessage, normalizeGitOpResult } from "./git-op.js";
import type { BackgroundTask, GitOpKickoff } from "./api.js";

function task(over: Partial<BackgroundTask>): BackgroundTask {
  return {
    id: "t1",
    kind: "git",
    label: "merge",
    status: "done",
    progress: null,
    startedAt: 0,
    endedAt: 1,
    error: null,
    ...over,
  } as BackgroundTask;
}

describe("normalizeGitOpResult", () => {
  test("returns the task's result payload when present", () => {
    const r = { success: true, stdout: "ok", stderr: "", status: 0 };
    expect(normalizeGitOpResult(task({ result: r }))).toEqual(r);
  });

  test("synthesizes success from a done task with no result", () => {
    const r = normalizeGitOpResult(task({ status: "done", result: undefined }));
    expect(r.success).toBe(true);
    expect(r.status).toBeNull();
  });

  test("synthesizes failure + carries task.error into stderr", () => {
    const r = normalizeGitOpResult(task({ status: "failed", error: "boom", result: undefined }));
    expect(r.success).toBe(false);
    expect(r.stderr).toBe("boom");
  });

  test("a null task is a failure", () => {
    const r = normalizeGitOpResult(null);
    expect(r.success).toBe(false);
    expect(r.stderr).toBe("");
  });
});

describe("awaitGitOp", () => {
  test("awaits the kickoff and normalizes", async () => {
    const kickoff: GitOpKickoff = {
      taskId: "t1",
      awaitDone: Promise.resolve(task({ result: { success: true, stdout: "", stderr: "", status: 0 } })),
    };
    const r = await awaitGitOp(kickoff);
    expect(r.success).toBe(true);
  });
});

describe("gitOpErrorMessage", () => {
  test("prefers stderr, then stdout, then the fallback", () => {
    expect(gitOpErrorMessage({ success: false, stdout: "out", stderr: "err", status: 1 }, "fb")).toBe("err");
    expect(gitOpErrorMessage({ success: false, stdout: "out", stderr: "", status: 1 }, "fb")).toBe("out");
    expect(gitOpErrorMessage({ success: false, stdout: "", stderr: "", status: 1 }, "fb")).toBe("fb");
  });

  test("trims whitespace", () => {
    expect(gitOpErrorMessage({ success: false, stdout: "", stderr: "  oops\n", status: 1 }, "fb")).toBe("oops");
  });
});
