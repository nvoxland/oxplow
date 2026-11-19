import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { SqlQueryResult } from "../../tauri-bridge/generated/bindings.js";

const ran: Array<[string, unknown]> = [];
let items: unknown[][] = [];
let baskets: unknown[][] = [];
let sent: unknown[][] = [];
const realApi = { ...(await import("../../api.js")) }; // a copy: `mock.module` patches a loaded module in place
mock.module("../../api.js", () => ({
  ...realApi,
  querySql: async (sql: string) =>
    ({
      columns: sql.includes("v_context_sent") ? ["basket", "items"] : sql.includes("v_context_basket") ? ["id", "name", "next_thread_id"] : ["id", "kind", "source_uri", "label", "text", "note", "basket"],
      rows: sql.includes("v_context_sent") ? sent : sql.includes("v_context_basket") ? baskets : items,
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
  sent = [];
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
  expect(view.getAllByTestId("context-panel-preview")[1]!.textContent).toBe("fn main() {}");
  fireEvent.click(view.getAllByTestId("context-panel-remove")[0]!);
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.remove", { item: 1 }]));
  fireEvent.click(view.getByTestId("context-panel-clear"));
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.clear", { thread: "thr7" }]));
  // The Context page opens from the panel's header, not a link here.
  expect(view.queryByTestId("context-panel-open-page")).toBeNull();
  expect(opened).toEqual([]);
});

// A row reads: the name, the note mark, the text; the × that takes it out
// ends the line, so every × lines up at the right.
test("a row's note mark leads its text and its × is at the right", async () => {
  setSelectedThread("thr7");
  items = [[1, "page", "oxplow:oxplow/commit?sha=abc1234", "abc1234", null, null, null]];
  const view = render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
  const item = await waitFor(() => view.getByTestId("context-panel-item"));
  expect(item.lastElementChild?.getAttribute("data-testid")).toBe("context-panel-remove");
  const ids = Array.from(item.children).map((c) => c.getAttribute("data-testid")).filter(Boolean);
  expect(ids).toEqual(["context-panel-title", "context-panel-note-mark", "context-panel-preview", "context-panel-remove"]);
});

// A row is the item's name, a note mark and its text on one line; its click
// expands it in place to the whole text, the note edited in place
// (`oxplow.context.note`) and Open; another click folds it. Folded, the
// details are inert.
test("a row expands in place to its whole text and its note", async () => {
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
  const rows = view.getAllByTestId("context-panel-item");
  const details = view.getAllByTestId("context-item-details");
  expect(details.map((d) => d.hasAttribute("inert"))).toEqual([true, true]);
  fireEvent.click(rows[0]!);
  expect(rows[0]!.getAttribute("aria-expanded")).toBe("true");
  expect(details[0]!.hasAttribute("inert")).toBe(false);
  expect(view.getAllByTestId("context-item-text")[0]!.textContent).toBe("fn main() {\n  run();\n}");
  expect(view.getAllByTestId("context-panel-note")[0]!.textContent).toBe("Handle the empty case.");
  fireEvent.click(view.getAllByTestId("context-item-open")[0]!);
  expect(opened).toEqual(["oxplow:oxplow/file?path=a.rs#lines=3-4"]);
  // One open at a time: the second row's click folds the first.
  fireEvent.click(rows[1]!);
  expect(rows[0]!.getAttribute("aria-expanded")).toBe("false");
  const note = view.getAllByTestId("context-panel-note")[1]!;
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

// One tab per basket, with its count; the one the next prompt sends wears a
// paper plane. Send Next on a named basket makes it the one; on, pressing it
// sends This Thread again. An empty basket can't be sent.
test("each basket is a tab, and Send Next picks the one the next prompt sends", async () => {
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
  // One marker: Auth goes next, so This Thread's switch is off.
  expect(tabs.map((t) => !!t.querySelector('[data-testid="context-tab-sending"]'))).toEqual([false, true, false, false]);
  expect(view.getByTestId("context-send-next").getAttribute("aria-checked")).toBe("false");
  fireEvent.click(view.getByTestId("context-send-next"));
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.send_next", { thread: "thr7" }]));
  fireEvent.click(tabs[1]!);
  await waitFor(() => expect(view.getByText("In Auth")).toBeTruthy());
  expect(view.getByTestId("context-send-next").getAttribute("aria-checked")).toBe("true");
  fireEvent.click(view.getByTestId("context-send-next"));
  await waitFor(() => expect(ran.filter(([n]) => n === "oxplow.context.send_next").length).toBe(2));
  fireEvent.click(tabs[2]!);
  await waitFor(() => expect(view.getByText("In Release")).toBeTruthy());
  expect(view.getByTestId("context-send-next").getAttribute("aria-checked")).toBe("false");
  fireEvent.click(view.getByTestId("context-send-next"));
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.send_next", { thread: "thr7", basket: "Release" }]));
  fireEvent.click(tabs[3]!);
  await waitFor(() => expect((view.getByTestId("context-send-next") as HTMLButtonElement).disabled).toBe(true));
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

// The bar says what the thread's last send carried, and Put Back returns it.
test("the last send is named on the bar and put back from it", async () => {
  setSelectedThread("thr7");
  baskets = [[1, "Auth", null]];
  sent = [["Auth", 2]];
  const view = render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
  const last = await waitFor(() => view.getByTestId("context-last-send"));
  expect(last.textContent).toContain("Sent 2 items from Auth");
  fireEvent.click(view.getByTestId("context-put-back"));
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.put_back", { thread: "thr7" }]));
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});
