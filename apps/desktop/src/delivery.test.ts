import { expect, spyOn, test } from "bun:test";

import * as api from "./api.js";
import { deliveryAlert, lettersFromResult, readFailedReactions } from "./delivery.js";
import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

const result = (rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({
    columns: ["id", "consumer", "event_seq", "event_type", "error", "attempts", "last_failed_at"],
    rows,
    truncated: false,
    reads: { datasets: ["v_event_dead_letter"], tables: [], measures: [] },
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

test("the Alerts row counts the undelivered events, or isn't there", () => {
  expect(deliveryAlert(0)).toBeNull();
  expect(deliveryAlert(1)).toBe("1 event couldn't be delivered");
  expect(deliveryAlert(5)).toBe("5 events couldn't be delivered");
});

test("readFailedReactions reads the latest retryable attempts and keeps their outcomes", async () => {
  const result = {
    columns: ["effect", "event_id", "event_seq", "attempt", "outcome", "reason", "event_type", "retry_at", "finished_at"],
    rows: [
      ["acme/mark-done", "e2", 42, 2, "interrupted", "A step outside oxplow may have run", "task.state_changed", null, "2026-11-24T10:00:00Z"],
      ["acme/assign", "e1", 41, 1, "denied", "Missing grant", "task.state_changed", null, null],
    ],
    truncated: false,
    reads: { datasets: ["v_effect_run", "v_event"], tables: [], measures: [] },
    freshness: {},
  } as unknown as SqlQueryResult;
  const query = spyOn(api, "querySql").mockResolvedValue(result);
  try {
    const { reactions, reads } = await readFailedReactions();
    expect(query).toHaveBeenCalledWith(expect.stringContaining("WHERE r.latest = 1 AND r.retryable = 1"), [], 200);
    expect(reactions).toEqual([
      {
        effect: "acme/mark-done",
        eventId: "e2",
        eventSeq: 42,
        attempt: 2,
        outcome: "interrupted",
        reason: "A step outside oxplow may have run",
        eventType: "task.state_changed",
        retryAt: null,
        failedAt: "2026-11-24T10:00:00Z",
      },
      {
        effect: "acme/assign",
        eventId: "e1",
        eventSeq: 41,
        attempt: 1,
        outcome: "denied",
        reason: "Missing grant",
        eventType: "task.state_changed",
        retryAt: null,
        failedAt: null,
      },
    ]);
    expect(reads).toEqual(result.reads);
  } finally {
    query.mockRestore();
  }
});
