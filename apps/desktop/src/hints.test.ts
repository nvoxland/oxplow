import { expect, test } from "bun:test";

import { hintsFromResult } from "./hints.js";

// The person's raised hints, as Alerts lists them.
test("hints read from their nudge rows", () => {
  expect(
    hintsFromResult({
      columns: ["id", "kind", "message", "thread_title", "created_at"],
      rows: [
        [4, "oxplow/landed-in-progress", "“t” is still in progress", "Main", "2026-11-24T10:00:00Z"],
        [2, "hint-muted", "muted", null, null],
      ],
      reads: { datasets: [], sources: [] },
      truncated: false,
    } as never),
  ).toEqual([
    { id: 4, kind: "oxplow/landed-in-progress", message: "“t” is still in progress", threadTitle: "Main", createdAt: "2026-11-24T10:00:00Z" },
    { id: 2, kind: "hint-muted", message: "muted", threadTitle: null, createdAt: null },
  ]);
});
