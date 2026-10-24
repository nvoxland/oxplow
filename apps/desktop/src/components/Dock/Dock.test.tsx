import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render } from "@testing-library/react";

const realApi = await import("../../api.js");
mock.module("../../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  subscribeOxplowEvents: () => () => {},
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} }),
}));
const { Dock } = await import("./Dock.js");
const { dockStore } = await import("./dockStore.js");
const { PanelRunsProvider } = await import("../Panels/PanelRunsContext.js");
const { alertsRef } = await import("../../tabs/pageRefs.js");

afterEach(() => {
  cleanup();
  dockStore.close();
});

function mount(onOpenPage: (r: unknown) => void = () => {}) {
  return render(
    <PanelRunsProvider streamId={null} threadId={null}>
      <Dock streamId={null} onOpenPage={onOpenPage}>
        <span data-testid="status">status</span>
      </Dock>
    </PanelRunsProvider>,
  );
}

test("the bell's icon opens the Alerts panel, and clicking it again closes it", () => {
  const view = mount();
  expect(view.queryByTestId("dock-panel")).toBeNull();
  const button = view.getByTestId("dock-alerts");
  expect(button.getAttribute("aria-pressed")).toBe("false");
  fireEvent.click(button);
  expect(view.getByTestId("dock-panel-alerts")).toBeTruthy();
  expect(button.getAttribute("aria-pressed")).toBe("true");
  expect(view.getByTestId("alerts-panel-empty").textContent).toBe("Nothing needs you.");
  fireEvent.click(button);
  expect(view.queryByTestId("dock-panel")).toBeNull();
});

test("the panel offers Open Alerts page, and × closes it", () => {
  const opened: unknown[] = [];
  const view = mount((r) => opened.push(r));
  fireEvent.click(view.getByTestId("dock-alerts"));
  fireEvent.click(view.getByTestId("alerts-panel-open-page"));
  expect(opened).toEqual([alertsRef()]);
  fireEvent.click(view.getByTestId("dock-panel-close"));
  expect(view.queryByTestId("dock-panel")).toBeNull();
});

test("Escape closes the panel and focus returns to its icon", () => {
  const view = mount();
  fireEvent.click(view.getByTestId("dock-alerts"));
  const panel = view.getByTestId("dock-panel");
  expect(document.activeElement).toBe(panel);
  let reachedOutside = false;
  document.addEventListener("keydown", () => (reachedOutside = true), { once: true });
  fireEvent.keyDown(panel, { key: "Escape" });
  expect(view.queryByTestId("dock-panel")).toBeNull();
  expect(document.activeElement).toBe(view.getByTestId("dock-alerts"));
  expect(reachedOutside).toBe(false);
});

test("the open state is in the store, so it survives the panel remounting", () => {
  const view = mount();
  act(() => dockStore.open("alerts"));
  expect(view.getByTestId("dock-panel")).toBeTruthy();
  view.unmount();
  const again = mount();
  expect(again.getByTestId("dock-panel-alerts")).toBeTruthy();
});
