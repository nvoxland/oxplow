import { afterEach, expect, test } from "bun:test";
import { act, cleanup, render } from "@testing-library/react";

import { useCoarsePointer } from "./coarsePointer.js";

afterEach(cleanup);

/** A media query list whose answer the test sets, and tells its listeners. */
function fakeQuery(matches: boolean) {
  const listeners = new Set<() => void>();
  const list = {
    get matches() {
      return matches;
    },
    addEventListener: (_: string, fn: () => void) => listeners.add(fn),
    removeEventListener: (_: string, fn: () => void) => listeners.delete(fn),
  };
  return {
    match: (query: string) => {
      expect(query).toBe("(pointer: coarse)");
      return list as unknown as MediaQueryList;
    },
    set(next: boolean) {
      matches = next;
      for (const fn of listeners) fn();
    },
    listeners,
  };
}

function Probe({ match }: { match: (q: string) => MediaQueryList }) {
  return <div data-testid="coarse">{String(useCoarsePointer(match))}</div>;
}

test("a touch screen's pointer is coarse, and the answer follows a change", () => {
  const q = fakeQuery(true);
  const view = render(<Probe match={q.match} />);
  expect(view.getByTestId("coarse").textContent).toBe("true");
  act(() => q.set(false));
  expect(view.getByTestId("coarse").textContent).toBe("false");
  view.unmount();
  expect(q.listeners.size).toBe(0);
});
