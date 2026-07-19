import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

// tsk181: picking a breakdown dimension with no rows used to unmount the whole
// card — INCLUDING its own dimension <select> — because `if (rows.length === 0)
// return null` sat above the picker. The page keeps the Breakdown tab selected,
// so the result was a blank tab with no control to choose a different
// dimension; the only way out was closing and reopening the tab.
//
// This is not an exotic state: any dimension can come back empty for the
// current range or branch, so a perfectly valid choice could trap you.

const realApi = await import("../api.js");

// `model` has data; `agent` has none — the exact pair that surfaced this.
mock.module("../api.js", () => ({
  ...realApi,
  metricDimensionRollup: async (_key: string, dim: string) =>
    dim === "model"
      ? [{ key: "claude-opus-4-8", value: 1234, subject_count: 2 }]
      : [],
}));

const { MetricBreakdownCard } = await import("./MetricDetail.js");

const def = (dims: string[]) =>
  ({
    key: "agent.tokens.output",
    title: "Output tokens",
    sliceable_dims_json: JSON.stringify(dims),
  }) as never;

afterEach(cleanup);

test("an empty dimension keeps the picker so you can choose another", async () => {
  // First declared dim is `agent`, which returns nothing.
  const view = render(<MetricBreakdownCard def={def(["agent", "model"])} />);

  await waitFor(() => expect(view.queryByTestId("breakdown-empty")).not.toBeNull());

  // The escape hatch must still be on screen.
  const picker = view.getByLabelText("Breakdown dimension") as HTMLSelectElement;
  expect(picker).not.toBeNull();
  expect(picker.value).toBe("agent");
  // ...and it must still offer the dimension that does have data.
  expect([...picker.options].map((o) => o.value)).toEqual(["agent", "model"]);
});

test("a dimension with rows renders them, not the empty state", async () => {
  const view = render(<MetricBreakdownCard def={def(["model"])} />);
  await waitFor(() => expect(view.queryByTestId("breakdown-row-claude-opus-4-8")).not.toBeNull());
  expect(view.queryByTestId("breakdown-empty")).toBeNull();
  expect(view.getByLabelText("Breakdown dimension")).not.toBeNull();
});

test("a metric that declares no dimensions renders no card at all", async () => {
  // The self-hide that SHOULD happen: nothing to break down by. Coverage and
  // most operational metrics land here.
  const view = render(<MetricBreakdownCard def={def([])} />);
  await waitFor(() => expect(view.queryByTestId("metric-breakdown")).toBeNull());
  expect(view.queryByTestId("breakdown-empty")).toBeNull();
});
