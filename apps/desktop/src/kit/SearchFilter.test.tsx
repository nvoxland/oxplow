import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { Toolbar } from "./Toolbar.js";
import { useSearchFilter } from "./SearchFilter.js";

afterEach(cleanup);

function List() {
  const filter = useSearchFilter("q", { label: "Filter", testId: "f" });
  return (
    <>
      <Toolbar filter={filter} />
      <span data-testid="seen">{filter.value}</span>
    </>
  );
}

test("the toolbar's search box is the tab's q view param", () => {
  const patches: Record<string, SqlCell | undefined>[] = [];
  const nav = {
    viewParams: { q: "foo" },
    patchViewParams: (patch: Record<string, SqlCell | undefined>) => patches.push(patch),
  } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <List />
    </PageNavigationContext.Provider>,
  );
  expect((view.getByTestId("f") as HTMLInputElement).value).toBe("foo");
  fireEvent.change(view.getByTestId("f"), { target: { value: "bar" } });
  fireEvent.change(view.getByTestId("f"), { target: { value: "" } });
  expect(patches).toEqual([{ q: "bar" }, { q: undefined }]);
});

test("outside a tab the search box still filters", () => {
  const view = render(<List />);
  fireEvent.change(view.getByTestId("f"), { target: { value: "x" } });
  expect(view.getByTestId("seen").textContent).toBe("x");
});
