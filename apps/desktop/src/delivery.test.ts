import { expect, test } from "bun:test";

import { deliveryAlert, lettersFromResult, letterLine, reactionLine, reactionsFromResult } from "./delivery.js";
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

// P9.D4: an effect's reaction that failed (its latest attempt) waits for a
// person — unless (P10) it is sent again by itself, which it says.
test("failed reactions read from v_effect_run, and say what failed on what", () => {
  const rows = {
    columns: ["effect", "event_id", "event_seq", "attempt", "reason", "event_type", "retry_at"],
    rows: [["acme/mark-done", "e1", 41, 1, "interrupted: a step outside oxplow may have run", "work_item.state_changed", null]],
    truncated: false,
    reads: { models: ["v_effect_run", "v_event"], tables: [], measures: [] },
    freshness: {},
  } as unknown as SqlQueryResult;
  const [r] = reactionsFromResult(rows);
  expect(r).toEqual({
    effect: "acme/mark-done",
    eventId: "e1",
    eventSeq: 41,
    attempt: 1,
    reason: "interrupted: a step outside oxplow may have run",
    eventType: "work_item.state_changed",
    retryAt: null,
  });
  expect(reactionLine(r)).toBe("acme/mark-done failed on work_item.state_changed (event 41)");
  expect(reactionLine({ ...r, attempt: 3 })).toBe("acme/mark-done failed on work_item.state_changed (event 41), attempt 3");
  expect(reactionLine({ ...r, retryAt: "2026-10-03T12:00:10.000Z" })).toBe(
    "acme/mark-done failed on work_item.state_changed (event 41); sent again by itself shortly",
  );
});
