import { afterEach, expect, test } from "bun:test";
import { act, cleanup, render } from "@testing-library/react";

import { createRememberedStore, RememberedContext } from "../../prefs/remembered.js";
import { DOCK_MIN_HEIGHT, useDockHeight } from "./dockStore.js";

afterEach(cleanup);

/** The dock's height is the device's app preference, clamped to the minimum. */
test("the dock's height is an app preference", async () => {
  const saves: [string, string, string | null][] = [];
  const store = createRememberedStore(
    {
      load: async () => [{ scope: "app", key: "dockHeight", valueJson: "320" }],
      save: async (scope, key, valueJson) => void saves.push([scope, key, valueJson]),
    },
    5,
  );
  let set: (px: number) => void = () => {};
  function Probe() {
    const [height, setHeight] = useDockHeight();
    set = setHeight;
    return <span data-testid="h">{height}</span>;
  }
  const view = render(
    <RememberedContext.Provider value={store}>
      <Probe />
    </RememberedContext.Provider>,
  );
  await act(async () => store.ready);
  expect(view.getByTestId("h").textContent).toBe("320");
  act(() => set(10));
  expect(view.getByTestId("h").textContent).toBe(String(DOCK_MIN_HEIGHT));
  await new Promise((r) => setTimeout(r, 20));
  expect(saves).toEqual([["app", "dockHeight", String(DOCK_MIN_HEIGHT)]]);
});
