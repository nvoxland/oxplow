import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "../../tabs/PageNavigationContext.js";
import { HierarchyView } from "./HierarchyView.js";

afterEach(cleanup);

const nodes = [
  { id: "a", label: "alpha.ts", children: [] },
  { id: "b", label: "beta.ts", children: [] },
];

// The search is the step's history state: Back and a restart keep it.
test("the search is read from, and written to, the step's history state", () => {
  const states: Record<string, unknown>[] = [];
  const nav = { viewParams: {}, patchViewParams() {}, historyState: { "t.search": "beta" }, setHistoryState: (p: Record<string, unknown>) => states.push(p) } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <HierarchyView nodes={nodes} testIdPrefix="t" />
    </PageNavigationContext.Provider>,
  );
  expect((view.getByTestId("t-search") as HTMLInputElement).value).toBe("beta");
  expect(view.queryByText("alpha.ts")).toBeNull();
  fireEvent.change(view.getByTestId("t-search"), { target: { value: "al" } });
  expect(states).toEqual([{ "t.search": "al" }]);
});
