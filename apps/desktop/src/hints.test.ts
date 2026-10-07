import { expect, test } from "bun:test";

import { hintsFromResult } from "./hints.js";

// The person's raised hints, as Alerts lists them.
test("hints read from their nudge rows", () => {
  expect(
    hintsFromResult({
      columns: ["id", "kind", "message", "thread_title"],
      rows: [
        [4, "oxplow-bundled/landed-in-progress", "“t” is still in progress", "Main"],
        [2, "hint-muted", "muted", null],
      ],
      reads: { models: [], sources: [] },
      truncated: false,
    } as never),
  ).toEqual([
    { id: 4, kind: "oxplow-bundled/landed-in-progress", message: "“t” is still in progress", threadTitle: "Main" },
    { id: 2, kind: "hint-muted", message: "muted", threadTitle: null },
  ]);
});
