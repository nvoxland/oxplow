import { describe, expect, test } from "bun:test";

import {
  ALL_DRAG_MIMES,
  RAIL_SECTION_DRAG_MIME,
  VIZ_ROW_DRAG_MIME,
} from "./dragMimes.js";

describe("drag MIME registry", () => {
  test("each MIME has its established literal value", () => {
    // Both ends of every drag live in this bundle, so these strings are
    // free to change — but a silent change breaks the source/sink pair
    // in a way nothing else would catch, so pin them.
    expect(RAIL_SECTION_DRAG_MIME).toBe("application/x-oxplow-rail-section");
    expect(VIZ_ROW_DRAG_MIME).toBe("application/x-oxplow-viz-rows");
  });

  test("every MIME is namespaced so foreign drags can't match", () => {
    // The whole point of a custom type: an OS file drag or a text drag
    // must never satisfy a drop target's `types.includes(...)` check.
    for (const mime of ALL_DRAG_MIMES) {
      expect(mime.startsWith("application/x-oxplow-")).toBe(true);
    }
  });

  test("no two drag kinds share a MIME", () => {
    // This registry exists because one MIME was once declared twice, in
    // two modules, with a test whose only job was to assert the copies
    // hadn't drifted. One home, one constant.
    expect(new Set(ALL_DRAG_MIMES).size).toBe(ALL_DRAG_MIMES.length);
  });
});
