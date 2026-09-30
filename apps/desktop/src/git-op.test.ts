import { describe, expect, test } from "bun:test";

import {
  awaitGitOp,
  gitOpErrorMessage,
  gitOpOutcomeMessage,
  normalizeGitOpResult,
  opErrorOf,
  settleGitOp,
} from "./git-op.js";
import type { BackgroundTask, GitOpKickoff, OpOutcome } from "./api.js";

function task(over: Partial<BackgroundTask>): BackgroundTask {
  return {
    id: "t1",
    kind: "vcs",
    label: "merge",
    status: "done",
    progress: null,
    startedAt: 0,
    endedAt: 1,
    error: null,
    ...over,
  } as BackgroundTask;
}

const outcome = (over: Partial<OpOutcome> = {}): OpOutcome => ({
  success: true,
  log: "",
  conflicts: [],
  auto_resolved: 0,
  ...over,
});

describe("normalizeGitOpResult", () => {
  test("returns the task's result payload when present", () => {
    const r = outcome({ log: "ok" });
    expect(normalizeGitOpResult(task({ result: r }))).toEqual(r);
  });

  test("synthesizes success from a done task with no result", () => {
    const r = normalizeGitOpResult(task({ status: "done", result: undefined }));
    expect(r.success).toBe(true);
    expect(r.conflicts).toEqual([]);
  });

  test("synthesizes failure + carries task.error into the log", () => {
    const r = normalizeGitOpResult(task({ status: "failed", error: "boom", result: undefined }));
    expect(r.success).toBe(false);
    expect(r.log).toBe("boom");
  });

  test("a null task is a failure", () => {
    const r = normalizeGitOpResult(null);
    expect(r.success).toBe(false);
    expect(r.log).toBe("");
  });
});

describe("awaitGitOp", () => {
  test("awaits the kickoff and normalizes", async () => {
    const kickoff: GitOpKickoff = {
      taskId: "t1",
      awaitDone: Promise.resolve(task({ result: outcome() })),
    };
    const r = await awaitGitOp(kickoff);
    expect(r.success).toBe(true);
  });
});

describe("gitOpErrorMessage", () => {
  test("names the conflicted paths, then the log, then the fallback", () => {
    expect(
      gitOpErrorMessage(outcome({ success: false, log: "CONFLICT", conflicts: ["a.ts", "b.ts"] }), "fb"),
    ).toBe("Conflicts in a.ts, b.ts");
    expect(gitOpErrorMessage(outcome({ success: false, log: "err" }), "fb")).toBe("err");
    expect(gitOpErrorMessage(outcome({ success: false }), "fb")).toBe("fb");
  });

  test("trims whitespace", () => {
    expect(gitOpErrorMessage(outcome({ success: false, log: "  oops\n" }), "fb")).toBe("oops");
  });
});

describe("gitOpOutcomeMessage", () => {
  test("plain success has no auto-resolve suffix", () => {
    expect(gitOpOutcomeMessage("Cherry-pick a1b2c3d", outcome())).toBe("Cherry-pick a1b2c3d succeeded");
  });

  test("success reports a singular auto-resolved conflict", () => {
    expect(gitOpOutcomeMessage("Revert a1b2c3d", outcome({ auto_resolved: 1 }))).toBe(
      "Revert a1b2c3d succeeded — 1 conflict auto-resolved",
    );
  });

  test("success pluralizes multiple auto-resolved conflicts", () => {
    expect(gitOpOutcomeMessage("Cherry-pick a1b2c3d", outcome({ auto_resolved: 3 }))).toBe(
      "Cherry-pick a1b2c3d succeeded — 3 conflicts auto-resolved",
    );
  });

  test("failure reports the op as failed", () => {
    expect(
      gitOpOutcomeMessage("Cherry-pick a1b2c3d", outcome({ success: false, log: "conflict" })),
    ).toBe("Cherry-pick a1b2c3d failed");
  });
});

describe("settleGitOp", () => {
  test("passes a command's outcome through", async () => {
    expect(await settleGitOp(async () => outcome({ log: "done" }))).toEqual(outcome({ log: "done" }));
  });

  test("a refused or failed command becomes an unsuccessful outcome with the reason", async () => {
    const r = await settleGitOp(async () => {
      throw new Error("`vcs.discard` needs confirmation");
    });
    expect(r.success).toBe(false);
    expect(r.log).toBe("`vcs.discard` needs confirmation");
  });
});

describe("opErrorOf", () => {
  test("carries the conflicts and the log as the detail", () => {
    expect(
      opErrorOf("Merge x", "merge x", outcome({ success: false, log: "CONFLICT\n", conflicts: ["a.ts"] })),
    ).toEqual({ label: "Merge x", command: "merge x", stderr: "Conflicts in a.ts\nCONFLICT", blankFailure: false });
  });

  test("flags a failure that says nothing", () => {
    expect(opErrorOf("Push", "push", outcome({ success: false })).blankFailure).toBe(true);
  });
});
