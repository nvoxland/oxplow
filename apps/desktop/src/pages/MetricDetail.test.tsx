import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { SeriesPoint } from "../api.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { RecordingsTable } from "./MetricDetail.js";

afterEach(cleanup);

const samples = Array.from({ length: 60 }, (_, i) => ({ capture_id: i, captured_at: "2026-06-01T00:00:00Z", value: i, branch: "main" }) as unknown as SeriesPoint);

function nav(historyState: Record<string, unknown>, writes: Record<string, unknown>[]): PageNavigation {
  return { historyState, setHistoryState: (patch: Record<string, unknown>) => writes.push(patch) } as unknown as PageNavigation;
}

test("the recordings page is the step's history state: restored on mount, reset only by a new filter", () => {
  const writes: Record<string, unknown>[] = [];
  const at = (key: string, state: Record<string, unknown>) => (
    <PageNavigationContext.Provider value={nav(state, writes)}>
      <RecordingsTable samples={samples} filterKey={key} />
    </PageNavigationContext.Provider>
  );
  const view = render(at("7d||", { recordingsPage: 2 }));
  // Coming back to page 3 writes nothing: it is not reset by mounting.
  expect(writes).toEqual([]);
  expect(view.getByTestId("recordings-pager").textContent).toContain("51–60 of 60");
  fireEvent.click(view.getByText("‹ Prev"));
  expect(writes).toEqual([{ recordingsPage: 1 }]);
  writes.length = 0;
  view.rerender(at("30d||", { recordingsPage: 2 }));
  expect(writes).toEqual([{ recordingsPage: 0 }]);
});
