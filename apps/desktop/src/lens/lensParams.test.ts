import { expect, test } from "bun:test";

import { paramKind, paramOptions, paramOptionsSql } from "./lensParams.js";

// tsk1043: a lens's Task Id / Stream params took raw numbers; oxplow's id
// params pick from what exists instead.

test("oxplow's id params are pickers; anything else is typed", () => {
  expect(paramKind("ref")).toBe("work_item");
  expect(paramKind("effort_id")).toBe("effort");
  expect(paramKind("thread_id")).toBe("thread");
  expect(paramKind("stream_id")).toBe("stream");
  expect(paramKind("change_id")).toBeNull();
  expect(paramKind("days")).toBeNull();
});

test("each picker reads its model, newest first", () => {
  expect(paramOptionsSql("work_item")).toContain("FROM v_work_item");
  expect(paramOptionsSql("effort")).toContain("FROM v_effort");
  expect(paramOptionsSql("thread")).toContain("FROM v_thread");
  expect(paramOptionsSql("stream")).toContain("FROM v_stream");
});

test("options are the rows' value and label", () => {
  const result = {
    columns: ["value", "label"],
    rows: [
      [12, "Reject a negative discount"],
      [7, null],
    ],
    truncated: false,
    reads: { models: [], tables: [], measures: [] },
    freshness: [],
  };
  expect(paramOptions(result)).toEqual([
    { value: 12, label: "Reject a negative discount" },
    { value: 7, label: "7" },
  ]);
});
