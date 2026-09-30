import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { ModelLineage } from "./ModelLineage.js";
import { SaveAsLens } from "./ExploreDataPage.js";
import { keepBlockedReason } from "./exploreData.js";

afterEach(cleanup);

test("lineage shows models as links, tables as text, and what reads it", () => {
  const onPick = mock((_: string) => {});
  const { getByTestId, queryByTestId } = render(
    <ModelLineage
      lineage={{ reads: [{ name: "claim", kind: "source" }, { name: "v_test_run", kind: "ref" }], readBy: ["v_effort_claim"] }}
      onPick={onPick}
    />,
  );
  fireEvent.click(getByTestId("explore-lineage-reads-v_test_run"));
  fireEvent.click(getByTestId("explore-lineage-read-by-v_effort_claim"));
  expect(onPick.mock.calls.map((c) => c[0])).toEqual(["v_test_run", "v_effort_claim"]);
  expect(getByTestId("explore-lineage-table-claim").tagName).toBe("CODE");
  expect(queryByTestId("explore-lineage-reads-claim")).toBeNull();
});

test("an empty lineage says so", () => {
  const { getByTestId } = render(<ModelLineage lineage={{ reads: [], readBy: [] }} onPick={() => {}} />);
  expect(getByTestId("explore-lineage").textContent).toContain("No other model.");
});

test("a raw read can't be saved as a lens: the button is off and says why", () => {
  const props = { query: "SELECT * FROM task", viz: "table" as const, stream: null, onOpenPage: () => {} };
  const raw = render(<SaveAsLens {...props} disabledReason={keepBlockedReason(true)} />);
  const button = raw.getByTestId("explore-save-open") as HTMLButtonElement;
  expect(button.disabled).toBe(true);
  expect(button.title).toContain("raw tables");
  fireEvent.click(button);
  expect(raw.queryByTestId("explore-save-title")).toBeNull();
  cleanup();

  const models = render(<SaveAsLens {...props} disabledReason={keepBlockedReason(false)} />);
  fireEvent.click(models.getByTestId("explore-save-open"));
  expect(models.getByTestId("explore-save-title")).toBeTruthy();
});
