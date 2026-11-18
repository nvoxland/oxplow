import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

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
  runCommand: async (name: string, input: unknown) => {
    ran.push([name, input]);
    return { audit_id: 1, inverse: null, result: {} };
  },
}));
const { setSelectedThread } = await import("../../contextBasket.js");
const { ContextDockBody } = await import("./ContextDockPanel.js");

afterEach(() => {
  cleanup();
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
  expect(view.getByTestId("context-panel-thread").textContent).toBe("This Thread");
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

// An item's note shows under it and is edited in place: the edit is
// `oxplow.context.note`, and an item with none offers "Add a note…".
test("an item's note shows under it and is edited in place", async () => {
  setSelectedThread("thr7");
  items = [
    [1, "text", "oxplow:oxplow/file?path=a.rs", null, "fn main() {}", "Handle the empty case.", null],
    [2, "page", "oxplow:oxplow/page?page=tasks", "Tasks", null, null, null],
  ];
  const view = render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getAllByTestId("context-panel-item").length).toBe(2));
  const notes = view.getAllByTestId("context-panel-note");
  expect(notes[0]!.textContent).toBe("Handle the empty case.");
  expect(notes[1]!.textContent).toBe("Add a note…");
  fireEvent.click(notes[1]!);
  const field = await waitFor(() => {
    const el = view.container.querySelector('textarea[data-testid="context-panel-note"]');
    if (!el) throw new Error("no note field yet");
    return el;
  });
  fireEvent.change(field, { target: { value: "Is this list right?" } });
  fireEvent.keyDown(field, { key: "Enter", metaKey: true });
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.note", { item: 2, note: "Is this list right?" }]));
});

test("with nothing anywhere it says how things get in", async () => {
  setSelectedThread("thr7");
  const view = render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getByTestId("context-panel-empty").textContent).toBe("No context added yet"));
  expect(view.queryByTestId("context-panel-clear")).toBeNull();
});

// A basket attached here is under This Thread, open, with Detach; another
// is collapsed under Baskets with Attach, and opens to its items.
test("an attached basket shows under This Thread; another attaches from Baskets", async () => {
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
  const attached = await waitFor(() => view.getByTestId("context-basket-attached"));
  expect(attached.textContent).toContain("In Auth");
  fireEvent.click(view.getByTestId("context-basket-detach"));
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.detach", { basket: "Auth" }]));
  expect(view.queryByText("In Release")).toBeNull();
  const release = view.getAllByTestId("context-basket").find((b) => b.dataset.basket === "Release")!;
  fireEvent.click(release.querySelector('[data-testid="context-basket-toggle"]')!);
  await waitFor(() => expect(view.getByText("In Release")).toBeTruthy());
  const attach = view.getAllByTestId("context-basket-attach") as HTMLButtonElement[];
  expect(attach.map((b) => b.disabled)).toEqual([false, true]);
  fireEvent.click(attach[0]!);
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.attach", { basket: "Release", thread: "thr7" }]));
});

// An item's right-click moves it to another basket; a basket's deletes it.
test("an item moves from its right-click and a basket is deleted from its own", async () => {
  setSelectedThread("thr7");
  items = [[1, "page", "oxplow:oxplow/commit?sha=a", "Mine", null, null, null]];
  baskets = [[1, "Auth", null]];
  const view = render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
  fireEvent.contextMenu(await waitFor(() => view.getByTestId("context-panel-item")));
  fireEvent.click(await waitFor(() => view.getByTestId("menu-item-move")));
  fireEvent.click(await waitFor(() => view.getByTestId("menu-item-move-1")));
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.move", { item: 1, basket: "Auth" }]));
  fireEvent.contextMenu(view.getByTestId("context-basket"));
  fireEvent.click(await waitFor(() => view.getByTestId("menu-item-delete")));
  await waitFor(() => expect(ran).toContainEqual(["oxplow.context.delete_basket", { basket: "Auth" }]));
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});
