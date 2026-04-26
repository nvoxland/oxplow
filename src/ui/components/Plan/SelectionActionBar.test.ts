import { describe, expect, test } from "bun:test";
import {
  shouldShowSelectionActionBar,
  summarizeSelection,
} from "./SelectionActionBar.js";

// Pure helpers only — bun:test runs without a DOM. The render-level
// expectations (Clear button calls onClear, Delete is disabled when every
// marked item is in_progress) are covered by the e2e suite that mounts
// React. Same pattern as Slideover.test.ts.

describe("shouldShowSelectionActionBar", () => {
  test("hides when no rows are marked", () => {
    expect(shouldShowSelectionActionBar(0)).toBe(false);
  });

  test("shows when one row is marked", () => {
    expect(shouldShowSelectionActionBar(1)).toBe(true);
  });

  test("shows when many rows are marked", () => {
    expect(shouldShowSelectionActionBar(7)).toBe(true);
  });
});

describe("summarizeSelection", () => {
  test("singular phrasing for one item", () => {
    expect(summarizeSelection(1)).toBe("1 selected");
  });

  test("plural phrasing for several items", () => {
    expect(summarizeSelection(3)).toBe("3 selected");
  });

  test("zero is plural-like (not normally rendered, but defined)", () => {
    expect(summarizeSelection(0)).toBe("0 selected");
  });
});
