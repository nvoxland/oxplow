import { expect, test } from "bun:test";

import { deliveryAlert, lettersFromResult, letterLine } from "./delivery.js";
import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

const result = (rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({
    columns: ["id", "consumer", "event_seq", "event_type", "error", "attempts", "last_failed_at"],
    rows,
    truncated: false,
    reads: { models: ["v_event_dead_letter"], tables: [], measures: [] },
    freshness: {},
  }) as unknown as SqlQueryResult;

test("rows read as undelivered events", () => {
  expect(lettersFromResult(result([[7, "change.analyze", 41, "snapshot.taken", "boom", 3, "2026-10-01T00:00:00Z"]]))).toEqual([
    {
      id: 7,
      consumer: "change.analyze",
      eventSeq: 41,
      eventType: "snapshot.taken",
      error: "boom",
      attempts: 3,
      lastFailedAt: "2026-10-01T00:00:00Z",
    },
  ]);
});

test("a letter says which consumer couldn't take which event", () => {
  const [l] = lettersFromResult(result([[7, "change.analyze", 41, "snapshot.taken", "boom", 1, "t"]]));
  expect(letterLine(l)).toBe("change.analyze couldn't take snapshot.taken (event 41), once");
  expect(letterLine({ ...l, attempts: 4 })).toBe("change.analyze couldn't take snapshot.taken (event 41), 4 times");
});

test("the Alerts row counts the undelivered events, or isn't there", () => {
  expect(deliveryAlert(0)).toBeNull();
  expect(deliveryAlert(1)).toBe("1 event couldn't be delivered");
  expect(deliveryAlert(5)).toBe("5 events couldn't be delivered");
});
