import { describe, expect, test } from "bun:test";

import { collapseAgentStatusState, formatAgentStallAlert } from "./api.js";

describe("collapseAgentStatusState", () => {
  test("running maps to working", () => {
    expect(collapseAgentStatusState("running")).toBe("working");
  });

  test("stalled stays distinct so the dot can render a failure", () => {
    expect(collapseAgentStatusState("stalled")).toBe("stalled");
  });

  test("awaiting_user stays distinct so the dot can render 'waiting on you'", () => {
    expect(collapseAgentStatusState("awaiting_user")).toBe("awaiting");
  });

  test("everything else collapses to waiting", () => {
    for (const raw of ["idle", "stopped", "error", undefined]) {
      expect(collapseAgentStatusState(raw)).toBe("waiting");
    }
  });
});

describe("formatAgentStallAlert", () => {
  test("pluralizes tasks and rounds minutes", () => {
    expect(formatAgentStallAlert({ threadId: "thr1", inProgressCount: 2, waitingMs: 17 * 60_000 })).toBe(
      "Agent appears stalled: 2 in-progress tasks but no agent activity for 17 min",
    );
  });

  test("singular task and sub-minute waits clamp to 1 min", () => {
    expect(formatAgentStallAlert({ threadId: "thr1", inProgressCount: 1, waitingMs: 10_000 })).toBe(
      "Agent appears stalled: 1 in-progress task but no agent activity for 1 min",
    );
  });
});
