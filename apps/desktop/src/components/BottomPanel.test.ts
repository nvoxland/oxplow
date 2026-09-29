import { describe, expect, test } from "bun:test";
import { detail } from "./BottomPanel.js";

describe("agent activity rows", () => {
  test("a tool request names the tool, its target and a refusal", () => {
    expect(detail("tool.requested", { tool: "Edit", path: "src/a.rs", decision: "allowed" })).toBe(
      "Edit · src/a.rs",
    );
    expect(
      detail("tool.requested", { tool: "Write", path: "b.rs", decision: "denied", reason: "no effort" }),
    ).toBe("Write · b.rs · denied: no effort");
  });
  test("a finished tool says how it went; a status says where it moved", () => {
    expect(detail("tool.finished", { tool: "Bash", detail: "cargo test", ok: false })).toBe(
      "Bash · cargo test · failed",
    );
    expect(detail("status.changed", { state: "awaiting_user", detail: "Pick A or B?" })).toBe(
      "awaiting_user · Pick A or B?",
    );
    expect(detail("turn.ended", { outcome: "interrupted" })).toBe("interrupted");
    expect(detail("prompt.submitted", { reprompt: true })).toBe("re-prompt");
    expect(detail("session.started", { harness: "claude", resumed: true })).toBe("claude (resumed)");
  });
  test("an unknown kind or payload shows nothing rather than throwing", () => {
    expect(detail("tool.finished", null)).toBe("");
    expect(detail("something.else", { x: 1 })).toBe("");
  });
});
