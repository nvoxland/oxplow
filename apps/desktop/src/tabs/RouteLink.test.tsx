import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { metricRef } from "./pageRefs.js";
import { RouteLink } from "./RouteLink.js";
import type { TabRef } from "./tabState.js";

afterEach(cleanup);

// Regression: RouteLink's destination prop must NOT be named `ref` — React 18
// strips a `ref` prop from function components, so it never reaches the
// component and `useRouteDispatch` gets `undefined` (crashing on `ref.id` at
// click). With `to`, the TabRef arrives and a plain click navigates.
test("clicking a RouteLink navigates to its `to` target", () => {
  const target = metricRef("oxplow.tokens.total");
  const calls: TabRef[] = [];
  const { getByTestId } = render(
    <RouteLink to={target} onNavigate={(r) => calls.push(r)} testId="route-link">
      Total tokens
    </RouteLink>,
  );
  fireEvent.click(getByTestId("route-link"));
  expect(calls).toEqual([target]);
});
