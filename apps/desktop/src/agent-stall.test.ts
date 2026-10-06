import { describe, expect, test } from "bun:test";

import { collapseAgentStatusState } from "./api.js";

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

