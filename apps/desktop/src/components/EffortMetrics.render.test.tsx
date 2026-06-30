import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { EffortMetricsBlock } from "./EffortMetrics.js";

afterEach(cleanup);

// With no backend in the test env the fetch rejects → no deltas.
test("with showWhenEmpty and no deltas, renders an explicit empty state", () => {
  const { getByTestId } = render(
    <EffortMetricsBlock
      effortId="eff1"
      startedAt="2026-06-30T00:00:00Z"
      endedAt={null}
      showWhenEmpty
    />,
  );
  expect(getByTestId("effort-metrics-empty-eff1").textContent).toContain(
    "No metrics collected",
  );
});

test("without showWhenEmpty and no deltas, renders nothing", () => {
  const { queryByTestId } = render(
    <EffortMetricsBlock effortId="eff1" startedAt="2026-06-30T00:00:00Z" endedAt={null} />,
  );
  expect(queryByTestId("effort-metrics-empty-eff1")).toBeNull();
  expect(queryByTestId("effort-metrics-eff1")).toBeNull();
});
