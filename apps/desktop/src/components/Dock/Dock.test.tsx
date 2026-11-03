import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render } from "@testing-library/react";

const realApi = await import("../../api.js");
mock.module("../../api.js", () => ({
  ...realApi,
  listExtensions: async () => [],
  subscribeOxplowEvents: () => () => {},
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }),
}));
const { Dock } = await import("./Dock.js");
const { dockStore } = await import("./dockStore.js");
const { PanelRunsProvider } = await import("../Panels/PanelRunsContext.js");
const { alertsRef, contextRef } = await import("../../tabs/pageRefs.js");

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

test("× closes the panel", () => {
  const view = mount();
  fireEvent.click(view.getByTestId("dock-alerts"));
  fireEvent.click(view.getByTestId("dock-panel-close"));
  expect(view.queryByTestId("dock-panel")).toBeNull();
});

// A panel whose things have a page of their own carries the open arrow in
// its header, as a section heading does (`OpenPageLink`), not a link in
// its body. A page opened from a panel would sit behind it: opening one
// closes the panel, so the page is what the person sees.
test("the panel's header arrow opens its page and closes the panel", () => {
  const opened: unknown[] = [];
  const view = mount((r) => opened.push(r));
  fireEvent.click(view.getByTestId("dock-alerts"));
  expect(view.queryByTestId("alerts-panel-open-page")).toBeNull();
  const arrow = view.getByTestId("dock-panel-open-page").querySelector("[data-testid=open-page-link]")!;
  expect(arrow.getAttribute("aria-label")).toBe("Open Alerts page");
  fireEvent.click(arrow);
  expect(opened).toEqual([alertsRef()]);
  expect(view.queryByTestId("dock-panel")).toBeNull();
  fireEvent.click(view.getByTestId("dock-context"));
  fireEvent.click(view.getByTestId("dock-panel-open-page").querySelector("[data-testid=open-page-link]")!);
  expect(opened).toEqual([alertsRef(), contextRef()]);
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

// The icons are justified to the right of the bar, after the status
// content, as a tray is.
test("the dock's icons sit at the right of the bar, after the status content", () => {
  const view = mount();
  const status = view.getByTestId("status");
  const bar = view.getByTestId("dock-bar");
  expect(status.compareDocumentPosition(bar) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});

// A panel is a popup: a press anywhere outside it and its bar closes it,
// as a menu closes; a press inside it, or on the bar's icons, does not
// (the icon's own click toggles).
test("a press outside the open panel closes it; one inside keeps it", () => {
  const view = mount();
  fireEvent.click(view.getByTestId("dock-alerts"));
  fireEvent.pointerDown(view.getByTestId("dock-panel-alerts"));
  expect(view.getByTestId("dock-panel")).toBeTruthy();
  fireEvent.pointerDown(view.getByTestId("dock-bar"));
  expect(view.getByTestId("dock-panel")).toBeTruthy();
  fireEvent.pointerDown(document.body);
  expect(view.queryByTestId("dock-panel")).toBeNull();
});

// The panel's header carries the icon its bar button has, so the panel
// reads as that button's, opened.
test("the open panel's header shows its icon before its title", () => {
  const view = mount();
  fireEvent.click(view.getByTestId("dock-alerts"));
  const icon = view.getByTestId("dock-panel-icon");
  const title = view.getByTestId("dock-panel-title");
  expect(title.textContent).toBe("Alerts");
  expect(icon.compareDocumentPosition(title) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(view.getByTestId("dock-alerts").querySelector("svg")?.innerHTML).toBe(icon.querySelector("svg")?.innerHTML);
});
