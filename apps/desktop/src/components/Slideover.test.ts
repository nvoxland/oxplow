import { describe, expect, test } from "bun:test";
import { DEFAULT_PANEL_WIDTH, backdropShouldClose } from "./Slideover.js";

// Slideover is a thin React shell — its only non-trivial bit is the
// "click on backdrop closes, click on panel doesn't" rule. The
// renderer-level invariants (Escape closes, focus moves into the panel
// on open) are covered by the e2e suite — bun:test runs without a DOM
// and the rest of this codebase tests UI by exercising pure logic, not
// by mounting React.

describe("Slideover backdrop click rule", () => {
  test("closes when the click hits the backdrop element itself", () => {
    const backdrop = {};
    expect(backdropShouldClose({ target: backdrop, currentTarget: backdrop })).toBe(true);
  });

  test("does not close when the click bubbled from the panel", () => {
    const backdrop = {};
    const panel = {};
    expect(backdropShouldClose({ target: panel, currentTarget: backdrop })).toBe(false);
  });
});

describe("default panel width", () => {
  test("keeps a readable minimum on a phone and never passes the screen", () => {
    // min(38vw, 560px) is ~148px at 390px wide.
    expect(DEFAULT_PANEL_WIDTH).toBe("min(max(38vw, 360px), 560px, 100vw)");
  });
});
