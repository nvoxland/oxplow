import { describe, expect, test } from "bun:test";
import { backdropShouldClose } from "./Slideover.js";

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
