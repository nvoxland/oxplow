import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, render, waitFor } from "@testing-library/react";

// One load of the extensions (the main worktree's, for every stream),
// shared by every component that reads them while any is mounted, and one
// reload when they change.

const realApi = await import("./api.js");
let loads = 0;
let names = ["a"];
let lastArgs: unknown[] = [];
const listeners: Array<(e: Record<string, unknown>) => void> = [];
mock.module("./api.js", () => ({
  ...realApi,
  listExtensions: async (...args: unknown[]) => {
    loads++;
    lastArgs = args;
    return names.map((name) => ({ name, enabled: true, ui: { slots: [], commands: [], decorators: [] }, lenses: [] }));
  },
  subscribeOxplowEvents: (l: (e: Record<string, unknown>) => void) => {
    listeners.push(l);
    return () => listeners.splice(listeners.indexOf(l), 1);
  },
}));
const { useExtensions } = await import("./extensionsStore.js");

afterEach(() => {
  cleanup();
  loads = 0;
  names = ["a"];
});

function Names({ id }: { id: string }) {
  const exts = useExtensions();
  return <div data-testid={id}>{exts === null ? "…" : exts.map((e) => e.name).join(",")}</div>;
}

test("every reader shares one load and one event subscription", async () => {
  const view = render(
    <>
      <Names id="one" />
      <Names id="two" />
      <Names id="three" />
    </>,
  );
  await waitFor(() => expect(view.getByTestId("three").textContent).toBe("a"));
  expect(view.getByTestId("one").textContent).toBe("a");
  expect(loads).toBe(1);
  expect(lastArgs).toEqual([]);
  expect(listeners.length).toBe(1);
});

test("a change to the extensions reloads them once for every reader", async () => {
  const view = render(
    <>
      <Names id="one" />
      <Names id="two" />
    </>,
  );
  await waitFor(() => expect(view.getByTestId("two").textContent).toBe("a"));
  names = ["a", "b"];
  act(() => listeners.forEach((l) => l({ kind: "configChanged" })));
  await waitFor(() => expect(view.getByTestId("one").textContent).toBe("a,b"));
  expect(view.getByTestId("two").textContent).toBe("a,b");
  expect(loads).toBe(2);
});

test("nothing is kept once no reader is mounted", async () => {
  const view = render(<Names id="one" />);
  await waitFor(() => expect(view.getByTestId("one").textContent).toBe("a"));
  view.unmount();
  expect(listeners.length).toBe(0);
  names = ["c"];
  const again = render(<Names id="one" />);
  await waitFor(() => expect(again.getByTestId("one").textContent).toBe("c"));
});
