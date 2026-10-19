import { expect, test } from "bun:test";

import type { DataEntity } from "../tauri-bridge/generated/bindings.js";
import { entityRows, entitySummary } from "./dataSectionModel.js";

const entity = (name: string, owner: string, kind: string): DataEntity => ({
  name,
  owner,
  kind,
  description: `${name} rows`,
});

test("entityRows puts core first, formats counts and flags unsynced entities", () => {
  const rows = entityRows(
    [
      entity("v_github_pr", "github", "declared"),
      entity("v_work_item", "core", "sql"),
      entity("v_commit", "core", "sql"),
      entity("v_issues_issue", "issues", "entity"),
    ],
    { v_work_item: { rows: 12345 }, v_commit: { rows: 0 }, v_issues_issue: { rows: 7 } },
  );
  expect(rows.map((r) => r.name)).toEqual(["v_commit", "v_work_item", "v_github_pr", "v_issues_issue"]);
  expect(rows[1]!.rows).toBe(new Intl.NumberFormat().format(12345));
  expect(rows[0]!.rows).toBe("0");
  expect(rows[2]!.rows).toBe("Not synced yet");
  expect(rows[2]!.available).toBe(false);
  expect(rows[3]!.rows).toBe("7");
  expect(entitySummary(rows)).toBe("4 entities · 2 from extensions");
  expect(entitySummary(rows.slice(0, 1))).toBe("1 entity");
});

// tsk1065: counts load one model at a time after the list shows, so a
// model too big to count in time costs its own cell, not the list.
test("a count still loading says so, and one that failed reads as a dash with why", () => {
  const [counting, failed] = entityRows([entity("v_a", "core", "sql"), entity("v_b", "core", "sql")], {
    v_b: { error: "query_sql: timed out after 5s" },
  });
  expect([counting!.rows, counting!.rowsTitle]).toEqual(["Counting…", undefined]);
  expect(failed!.rows).toBe("—");
  expect(failed!.rowsTitle).toBe("Couldn't count it: query_sql: timed out after 5s");
});

import { backfillAsk, backfillDone, backfillRunLabel } from "./dataSectionModel.js";

// P9.D5: what a backfill would do, said before it runs, and what it did.
test("a backfill says how many events it would react to, and how it went", () => {
  expect(backfillAsk("acme/mark-done", 0)).toBe("acme/mark-done has reacted to every matching event: nothing to backfill.");
  expect(backfillAsk("acme/mark-done", 1)).toBe(
    "acme/mark-done never reacted to 1 matching event. Backfilling runs it on that event, as it is now; it may call outside oxplow.",
  );
  expect(backfillAsk("acme/mark-done", 3)).toBe(
    "acme/mark-done never reacted to 3 matching events. Backfilling runs it on each, oldest first, as it is now; it may call outside oxplow for every one.",
  );
  expect(backfillDone({ planned: 3, ran: 3, skipped: 0, proposed: 0, failed: 0, remaining: 0 })).toBe("Reacted to 3 of 3 events.");
  expect(backfillDone({ planned: 5, ran: 1, skipped: 1, proposed: 0, failed: 3, remaining: 0, stopped: "the effect was disabled: 3 failures in a row" })).toBe(
    "Reacted to 1 of 5 events; 3 failed, 1 skipped. Stopped: the effect was disabled: 3 failures in a row.",
  );
  expect(backfillDone({ planned: 300, ran: 200, skipped: 0, proposed: 0, failed: 0, remaining: 100 })).toBe(
    "Reacted to 200 of 300 events; 100 remain — run it again for the rest.",
  );
});

// tsk849: a run makes at most a batch of reactions; its button says so.
test("a backfill's button says how many one run reacts to", () => {
  expect(backfillRunLabel(1, 200)).toBe("Run on 1 event");
  expect(backfillRunLabel(200, 200)).toBe("Run on 200 events");
  expect(backfillRunLabel(500, 200)).toBe("Run on the first 200 of 500");
});


