import { afterEach, expect, test } from "bun:test";
import { act, cleanup, render } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "./PageNavigationContext.js";
import { useEntryState } from "./useEntryState.js";

afterEach(cleanup);

let setter: ((v: number) => void) | null = null;
function Probe() {
  const [n, set] = useEntryState<number>("n", 0);
  setter = set;
  return <span data-testid="n">{n}</span>;
}

function nav(state: Record<string, unknown>, writes: Record<string, unknown>[]): PageNavigation {
  return { state, setEntryState: (patch) => writes.push(patch) } as unknown as PageNavigation;
}

test("in a tab it reads the entry's value and writes the entry, once per change", () => {
  const writes: Record<string, unknown>[] = [];
  const view = render(
    <PageNavigationContext.Provider value={nav({ n: 5 }, writes)}>
      <Probe />
    </PageNavigationContext.Provider>,
  );
  expect(view.getByTestId("n").textContent).toBe("5");
  act(() => setter!(5));
  expect(writes).toEqual([]);
  act(() => setter!(7));
  expect(writes).toEqual([{ n: 7 }]);
});

test("another entry shows its own value", () => {
  const writes: Record<string, unknown>[] = [];
  const view = render(
    <PageNavigationContext.Provider value={nav({ n: 1 }, writes)}>
      <Probe />
    </PageNavigationContext.Provider>,
  );
  expect(view.getByTestId("n").textContent).toBe("1");
  view.rerender(
    <PageNavigationContext.Provider value={nav({ n: 2 }, writes)}>
      <Probe />
    </PageNavigationContext.Provider>,
  );
  expect(view.getByTestId("n").textContent).toBe("2");
});

test("outside a tab it is local state", () => {
  const view = render(<Probe />);
  expect(view.getByTestId("n").textContent).toBe("0");
  act(() => setter!(3));
  expect(view.getByTestId("n").textContent).toBe("3");
});
