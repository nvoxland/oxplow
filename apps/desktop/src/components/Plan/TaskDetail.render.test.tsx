import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import type { EffortDetail } from "../../api.js";
import { ActivityTimeline } from "./TaskDetail.js";

afterEach(cleanup);

function effort(overrides: Partial<EffortDetail["effort"]>): EffortDetail {
  return {
    effort: {
      id: "eff-1",
      task_id: "tsk-1",
      started_at: "2026-06-30T00:00:00Z",
      ended_at: null,
      start_snapshot_id: "10",
      end_snapshot_id: null,
      summary: null,
      ...overrides,
    },
    start_snapshot: null,
    end_snapshot: null,
    // Populated on purpose: an in-progress effort must NOT render these.
    changed_paths: ["src/a.ts", "src/b.ts"],
    counts: { created: 0, updated: 2, deleted: 0 },
  };
}

const noop = (iso: string) => iso;

test("an in-progress effort shows only 'In progress' — no files/tests/summary", () => {
  const { getByTestId, queryByText, queryByTestId } = render(
    <ActivityTimeline efforts={[effort({ ended_at: null })]} formatTimestamp={noop} />,
  );
  const section = getByTestId("tasks-effort-in-progress");
  expect(section.textContent).toContain("In progress");
  // None of the completed-effort breakdown leaks while in progress.
  expect(queryByText("Modified Files")).toBeNull();
  expect(queryByTestId("effort-observations-eff-1")).toBeNull();
  expect(queryByTestId("tasks-effort-summary-eff-1")).toBeNull();
  expect(queryByTestId("tasks-show-in-history-eff-1")).toBeNull();
});
