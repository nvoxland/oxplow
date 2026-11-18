import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { SqlQueryResult } from "../../tauri-bridge/generated/bindings.js";

const ran: Array<[string, unknown]> = [];
let items: unknown[][] = [];
let baskets: unknown[][] = [];
const realApi = { ...(await import("../../api.js")) }; // a copy: `mock.module` patches a loaded module in place
mock.module("../../api.js", () => ({
  ...realApi,
  querySql: async (sql: string) =>
    ({
      columns: sql.includes("v_context_basket") ? ["id", "name", "attached_thread_id"] : ["id", "kind", "source_uri", "label", "text", "note", "basket"],
      rows: sql.includes("v_context_basket") ? baskets : items,
      truncated: false,
      reads: { datasets: [], tables: [], measures: [] },
      freshness: {},
    }) as unknown as SqlQueryResult,
  getUiPreferences: async () => [],
  setUiPreference: async () => {},
  runCommand: async (name: string, input: unknown) => {
    ran.push([name, input]);
    return { audit_id: 1, inverse: null, result: {} };
  },
}));
const { addToContext, cancelAddToContext, setSelectedThread } = await import("../../contextBasket.js");
const { ContextDockBody } = await import("./ContextDockPanel.js");
const { resetAppRememberedStore } = await import("../../prefs/remembered.js");

afterEach(() => {
  cleanup();
  cancelAddToContext();
  resetAppRememberedStore();
  ran.length = 0;
  items = [];
  baskets = [];
  setSelectedThread(null);
});

test("it lists This Thread's items, takes one out and clears the lot", async () => {
  setSelectedThread("thr7");
  items = [
    [1, "page", "oxplow:oxplow/commit?sha=abc1234", "abc1234", null, null, null],
    [2, "text", "oxplow:oxplow/file?path=a.rs", null, "fn main() {}", null, null],
  ];
  const opened: string[] = [];
  const view = render(<ContextDockBody streamId={null} onOpenPage={(r) => opened.push(r.id)} />);
  await waitFor(() => expect(view.getAllByTestId("context-panel-item").length).toBe(2));
  expect(view.getAllByTestId("context-tab")[0]!.textContent).toBe("This Thread2");
  expect(view.getByText("abc1234")).toBeTruthy();
  expect(view.getByText("fn main() {}")).toBeTruthy();
  fireEvent.click(view.getAllByTestId("context-panel-remove")[0]!);
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.remove", { item: 1 }]));
  // Clear is a button, as the Alerts panel's.
  expect(view.getByTestId("context-panel-clear").className).toContain("ox-button");
  fireEvent.click(view.getByTestId("context-panel-clear"));
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.clear", { thread: "thr7" }]));
  // The Context page opens from the panel's header, not a link here.
  expect(view.queryByTestId("context-panel-open-page")).toBeNull();
  expect(opened).toEqual([]);
});

// The × that takes an item out leads its line, before the item's kind and
// title, so every × lines up at the left.
test("an item's × is at the left of its line", async () => {
  setSelectedThread("thr7");
  items = [[1, "page", "oxplow:oxplow/commit?sha=abc1234", "abc1234", null, null, null]];
  const view = render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
  const item = await waitFor(() => view.getByTestId("context-panel-item"));
  expect(item.firstElementChild?.getAttribute("data-testid")).toBe("context-panel-remove");
});

// A row is the item's name, its text on one line and a note mark; its
// click opens the item alone, where the whole text shows, the name opens its
// source and the note is edited in place (`oxplow.context.note`). Back
// returns to the list.
test("a row opens the item alone, with its whole text and its note", async () => {
  setSelectedThread("thr7");
  items = [
    [1, "text", "oxplow:oxplow/file?path=a.rs#lines=3-4", null, "fn main() {\n  run();\n}", "Handle the empty case.", null],
    [2, "page", "oxplow:oxplow/page?page=tasks", "Tasks", null, null, null],
  ];
  const opened: string[] = [];
  const view = render(<ContextDockBody streamId={null} onOpenPage={(r) => opened.push(r.id)} />);
  await waitFor(() => expect(view.getAllByTestId("context-panel-item").length).toBe(2));
  expect(view.getAllByTestId("context-panel-preview")[0]!.textContent).toBe("fn main() { run(); }");
  expect(view.getAllByTestId("context-panel-note-mark").map((m) => m.getAttribute("aria-label"))).toEqual(["Note: Handle the empty case.", "No note"]);
  fireEvent.click(view.getAllByTestId("context-panel-item")[0]!);
  expect(view.getByTestId("context-item-text").textContent).toBe("fn main() {\n  run();\n}");
  expect(view.getByTestId("context-panel-note").textContent).toBe("Handle the empty case.");
  fireEvent.click(view.getByTestId("context-item-title"));
  expect(opened).toEqual(["oxplow:oxplow/file?path=a.rs#lines=3-4"]);
  fireEvent.click(view.getByTestId("context-item-back"));
  fireEvent.click(view.getAllByTestId("context-panel-item")[1]!);
  const note = view.getByTestId("context-panel-note");
  expect(note.textContent).toBe("Add a note for the agent…");
  fireEvent.click(note);
  const field = await waitFor(() => {
    const el = view.container.querySelector('textarea[data-testid="context-panel-note"]');
    if (!el) throw new Error("no note field yet");
    return el;
  });
  fireEvent.change(field, { target: { value: "Is this list right?" } });
  fireEvent.keyDown(field, { key: "Enter", metaKey: true });
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.note", { item: 2, note: "Is this list right?" }]));
});

test("an empty basket says how things get in; with no thread and no basket the panel does", async () => {
  setSelectedThread("thr7");
  const view = render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getByTestId("context-tab-empty").textContent).toContain("Add a page from its ☰"));
  expect((view.getByTestId("context-panel-clear") as HTMLButtonElement).disabled).toBe(true);
  cleanup();
  setSelectedThread(null);
  const none = render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
  await waitFor(() => expect(none.getByTestId("context-panel-empty").textContent).toBe("No context added yet"));
});

// One tab per basket, with its count; a basket attached here wears a mark
// and This Thread's bar names it. A named basket's tab shows its items and
// its bar attaches or detaches it.
test("each basket is a tab; a named basket's bar attaches and detaches it", async () => {
  setSelectedThread("thr7");
  items = [
    [1, "page", "oxplow:oxplow/commit?sha=a", "In Auth", null, null, "Auth"],
    [2, "page", "oxplow:oxplow/commit?sha=b", "In Release", null, null, "Release"],
  ];
  baskets = [
    [1, "Auth", 7],
    [2, "Release", null],
    [3, "Empty", null],
  ];
  const view = render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getAllByTestId("context-tab").length).toBe(4));
  const tabs = view.getAllByTestId("context-tab");
  expect(tabs.map((t) => [t.dataset.basket, t.getAttribute("aria-selected"), t.querySelector('[data-testid="context-tab-count"]')!.textContent])).toEqual([
    ["@thread", "true", "0"],
    ["Auth", "false", "1"],
    ["Release", "false", "1"],
    ["Empty", "false", "0"],
  ]);
  expect(tabs[1]!.querySelector('[aria-label="Attached"]')).toBeTruthy();
  expect(view.getByTestId("context-tab-empty")).toBeTruthy();
  fireEvent.click(view.getByTestId("context-attached-link"));
  await waitFor(() => expect(view.getByText("In Auth")).toBeTruthy());
  // Attached here, the toggle is on; pressing it detaches.
  expect(view.getByTestId("context-basket-attach").getAttribute("aria-checked")).toBe("true");
  fireEvent.click(view.getByTestId("context-basket-attach"));
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.detach", { basket: "Auth" }]));
  fireEvent.click(tabs[2]!);
  await waitFor(() => expect(view.getByText("In Release")).toBeTruthy());
  expect(view.queryByText("In Auth")).toBeNull();
  expect(view.getByTestId("context-basket-attach").getAttribute("aria-checked")).toBe("false");
  fireEvent.click(view.getByTestId("context-basket-attach"));
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.attach", { basket: "Release", thread: "thr7" }]));
  fireEvent.click(tabs[3]!);
  await waitFor(() => expect((view.getByTestId("context-basket-attach") as HTMLButtonElement).disabled).toBe(true));
});

// An item's right-click moves it to another basket; a tab's deletes its basket.
test("an item moves from its right-click and a basket is deleted from its tab's", async () => {
  setSelectedThread("thr7");
  items = [[1, "page", "oxplow:oxplow/commit?sha=a", "Mine", null, null, null]];
  baskets = [[1, "Auth", null]];
  const view = render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
  fireEvent.contextMenu(await waitFor(() => view.getByTestId("context-panel-item")));
  fireEvent.click(await waitFor(() => view.getByTestId("menu-item-move")));
  fireEvent.click(await waitFor(() => view.getByTestId("menu-item-move-1")));
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.move", { item: 1, basket: "Auth" }]));
  const auth = view.getAllByTestId("context-tab").find((t) => t.dataset.basket === "Auth")!;
  fireEvent.contextMenu(auth);
  fireEvent.click(await waitFor(() => view.getByTestId("menu-item-delete")));
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.delete_basket", { basket: "Auth" }]));
});

// While an add waits the card is all the panel shows; the add opens the tab
// of the basket it went to.
test("an add hides the tabs, then opens the tab it went to", async () => {
  setSelectedThread("thr7");
  baskets = [[1, "Auth", null]];
  const view = render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getAllByTestId("context-tab").length).toBe(2));
  act(() => void addToContext({ uri: "oxplow:oxplow/commit?sha=c", label: "C" }));
  await waitFor(() => view.getByTestId("context-add-card"));
  expect(view.queryByTestId("context-tab")).toBeNull();
  fireEvent.click(view.getAllByTestId("context-add-to").find((c) => c.dataset.value === "Auth")!);
  fireEvent.click(view.getByTestId("context-note-add"));
  await waitFor(() => expect(view.getAllByTestId("context-tab").find((t) => t.dataset.basket === "Auth")!.getAttribute("aria-selected")).toBe("true"));
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});
