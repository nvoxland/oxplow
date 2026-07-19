import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

// tsk167: `Save` persists the dashboard's range, branch, AND dimension filter,
// but reopening the board dropped the filter every time.
//
// Two effects undid the hydration:
//  1. the "dimension changed -> clear its value" effect can't tell hydration
//     (null -> saved, in the same batched commit that restores the value) from
//     a user picking a new dimension, so it wiped the restored value;
//  2. the "dimension no longer offered -> drop it" effect validates against
//     `breakoutOptions`, which derives from `defs` — loaded on a SEPARATE
//     promise. When the dashboard resolves first the options are still empty,
//     so a perfectly valid saved dimension was discarded on load order alone.
//
// This test forces the worse ordering (defs resolve a macrotask AFTER the
// dashboard), which trips both, and asserts the filter reaches the tiles
// intact. The tile is stubbed to print the `groupFilter` it receives, so the
// assertion reads the value that actually scopes the tiles rather than a
// <select> whose option list is populated by the tiles themselves.

const realApi = await import("../api.js");

const SETTINGS = JSON.stringify({
  range: "all",
  filterDim: "package",
  filterValue: "oxplow-app",
});

const DASHBOARD = {
  dashboard: { id: "dsh1", title: "Board", settings_json: SETTINGS },
  items: [
    {
      id: "itm1",
      dashboard_id: "dsh1",
      kind: "metric",
      metric_key: "m.one",
      options_json: null,
      sort_index: 0,
    },
  ],
};

// Breakout options come from the spec's declared dims (tsk179), so the def has
// to declare "package" for it to be a legal option — once `defs` has loaded.
const DEFS = [
  { key: "m.one", title: "One", sliceable_dims_json: JSON.stringify(["package"]) },
];

mock.module("../api.js", () => ({
  ...realApi,
  getDashboard: async () => DASHBOARD,
  listMetricCatalog: async () => [],
  // Resolve on a later macrotask so the dashboard always wins the race.
  listMetricDefinitions: async () => {
    await new Promise((r) => setTimeout(r, 20));
    return DEFS;
  },
  subscribeOxplowEvents: () => () => {},
}));

mock.module("../components/Dashboard/MetricTile.js", () => ({
  MetricTile: ({ groupFilter }: { groupFilter: { dim: string | null; value: string | null } }) => (
    <div data-testid="tile-filter">{`${groupFilter.dim ?? "-"}|${groupFilter.value ?? "-"}`}</div>
  ),
}));

const { CustomDashboardPage } = await import("./CustomDashboardPage.js");

afterEach(cleanup);

test("a saved view restores its dimension filter even when defs load last", async () => {
  const view = render(<CustomDashboardPage dashboardId="dsh1" />);

  const tile = await view.findByTestId("tile-filter");
  // Before the fix this read "-|-" (dimension dropped against empty options)
  // and, once defs landed, "package|-" (value wiped by the reset effect).
  await waitFor(() => expect(tile.textContent).toBe("package|oxplow-app"));

  // The picker agrees once its options exist.
  const dim = await view.findByTestId("dashboard-filter-dim");
  expect((dim as HTMLSelectElement).value).toBe("package");

  // And it stays put after defs have settled — no late effect clears it.
  await new Promise((r) => setTimeout(r, 40));
  expect(tile.textContent).toBe("package|oxplow-app");
});

test("picking a different dimension still clears the old dimension's value", async () => {
  // The hydration guard must not disable the real behaviour: a USER change of
  // dimension invalidates the chosen value, which belongs to the old dimension.
  const view = render(<CustomDashboardPage dashboardId="dsh1" />);
  const tile = await view.findByTestId("tile-filter");
  await waitFor(() => expect(tile.textContent).toBe("package|oxplow-app"));

  const dim = (await view.findByTestId("dashboard-filter-dim")) as HTMLSelectElement;
  const { fireEvent } = await import("@testing-library/react");
  fireEvent.change(dim, { target: { value: "" } });

  await waitFor(() => expect(tile.textContent).toBe("-|-"));
});
