import { expect, test } from "bun:test";

import { openHandoffsFromResult } from "./handoffs.js";

test("open handoffs are keyed by thread id, with the newest sender", () => {
  const open = openHandoffsFromResult({
    columns: ["thread", "n", "newest_from"],
    rows: [
      ["thr3", 2, "Parser"],
      ["thr7", 1, "Docs"],
    ],
    truncated: false,
    reads: { models: ["v_handoff"], tables: [], measures: [] },
    freshness: {},
  } as never);
  expect(open).toEqual({
    thr3: { count: 2, newestFrom: "Parser" },
    thr7: { count: 1, newestFrom: "Docs" },
  });
});
