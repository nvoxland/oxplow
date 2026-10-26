import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { metricRef } from "./pageRefs.js";
import { setWindowRefOpener } from "./openRef.js";
import { PageNavigationContext } from "./PageNavigationContext.js";
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

// A ref string of any kind opens through the page's `openRef`: a plain
// click in this tab, a Cmd-click in a new one. Outside a page it opens
// through the window's opener.
test("a link to a ref string opens through openRef, and Cmd-click asks for a new tab", () => {
  const opened: Array<[string, boolean | undefined]> = [];
  const nav = {
    navigate: () => {},
    openRef: (ref: string, how?: { newTab?: boolean }) => opened.push([ref, how?.newTab]),
    goBack: () => {},
    goForward: () => {},
    canGoBack: false,
    canGoForward: false,
  };
  const { getByTestId } = render(
    <PageNavigationContext.Provider value={nav}>
      <RouteLink to="oxplow:oxplow/handoff?id=hnd1" testId="ref-link">
        A handoff
      </RouteLink>
    </PageNavigationContext.Provider>,
  );
  fireEvent.click(getByTestId("ref-link"));
  fireEvent.click(getByTestId("ref-link"), { metaKey: true });
  expect(opened).toEqual([
    ["oxplow:oxplow/handoff?id=hnd1", false],
    ["oxplow:oxplow/handoff?id=hnd1", true],
  ]);
});

test("outside a page a ref string opens through the window's opener", () => {
  const opened: string[] = [];
  setWindowRefOpener((ref) => opened.push(ref));
  const { getByTestId } = render(
    <RouteLink to="oxplow:oxplow/handoff?id=hnd3" testId="ref-link">
      A handoff
    </RouteLink>,
  );
  fireEvent.click(getByTestId("ref-link"));
  setWindowRefOpener(null);
  expect(opened).toEqual(["oxplow:oxplow/handoff?id=hnd3"]);
});
