import { expect, test } from "bun:test";
import type { EffortDetail, WorkNote } from "../../api.js";
import { buildActivityTimeline } from "./WorkItemDetail.js";

function note(id: string, created_at: string, body = "n"): WorkNote {
  return { id, work_item_id: "w1", body, author: "agent", created_at };
}

function effort(
  id: string,
  started_at: string,
  ended_at: string | null,
  summary: string | null = null,
): EffortDetail {
  return {
    effort: {
      id,
      work_item_id: "w1",
      started_at,
      ended_at,
      start_snapshot_id: null,
      end_snapshot_id: null,
      summary,
    },
    start_snapshot: null,
    end_snapshot: null,
    changed_paths: [],
    counts: { created: 0, updated: 0, deleted: 0 },
  };
}

test("buildActivityTimeline merges notes and efforts newest-first", () => {
  const notes = [
    note("n1", "2026-04-25T10:00:00Z", "first note"),
    note("n2", "2026-04-25T12:00:00Z", "later note"),
  ];
  const efforts = [
    effort("e1", "2026-04-25T09:00:00Z", "2026-04-25T11:00:00Z"),
    effort("e2", "2026-04-25T13:00:00Z", null),
  ];
  const rows = buildActivityTimeline(notes, efforts);
  // Expected order newest first by primary timestamp:
  //   e2 active (13:00), n2 (12:00), e1 ended (11:00), n1 (10:00)
  expect(rows.map((r) => r.kind + ":" + r.id)).toEqual([
    "effort:e2",
    "note:n2",
    "effort:e1",
    "note:n1",
  ]);
  expect(rows[0].active).toBe(true);
  expect(rows[2].active).toBe(false);
});

test("buildActivityTimeline uses ended_at as primary timestamp for closed efforts", () => {
  const notes: WorkNote[] = [];
  const efforts = [
    effort("e1", "2026-04-25T09:00:00Z", "2026-04-25T11:00:00Z"),
  ];
  const rows = buildActivityTimeline(notes, efforts);
  expect(rows[0].timestamp).toBe("2026-04-25T11:00:00Z");
});

test("buildActivityTimeline returns empty list when nothing recorded", () => {
  expect(buildActivityTimeline([], [])).toEqual([]);
});
