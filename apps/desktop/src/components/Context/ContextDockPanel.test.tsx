import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { SqlCell, SqlQueryResult } from "../../tauri-bridge/generated/bindings.js";

const ran: Array<[string, unknown]> = [];
let rows: SqlCell[][] = [];
const realApi = { ...(await import("../../api.js")) }; // a copy: `mock.module` patches a loaded module in place
mock.module("../../api.js", () => ({
  ...realApi,
  querySql: async () =>
    ({
      columns: ["id", "kind", "source_uri", "label", "text", "note", "delivered_at"],
      rows,
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
  setSelectedThread(null);
});

test("it lists what waits, takes an item out and clears the lot", async () => {
  setSelectedThread("thr7");
  rows = [
    [1, "page", "oxplow:oxplow/commit?sha=abc1234", "abc1234", null, null, null],
    [2, "text", "oxplow:oxplow/file?path=a.rs", null, "fn main() {}", null, null],
    [3, "page", "oxplow:oxplow/page?page=tasks", "Tasks", null, null, "2026-10-24T10:00:00Z"],
  ];
  const opened: string[] = [];
  const view = render(<ContextDockBody streamId={null} onOpenPage={(r) => opened.push(r.id)} />);
  await waitFor(() => expect(view.getAllByTestId("context-panel-item").length).toBe(2));
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
  rows = [[1, "page", "oxplow:oxplow/commit?sha=abc1234", "abc1234", null, null, null]];
  const view = render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
  const item = await waitFor(() => view.getByTestId("context-panel-item"));
  expect(item.firstElementChild?.getAttribute("data-testid")).toBe("context-panel-remove");
});

// A waiting item's note shows under it and is edited in place: the edit
// is `oxplow.context.note`, and an item with none offers "Add a note…".
test("an item's note shows under it and is edited in place", async () => {
  setSelectedThread("thr7");
  rows = [
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

test("an empty basket says how things get in", async () => {
  setSelectedThread("thr7");
  rows = [];
  const view = render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getByTestId("context-panel-empty").textContent).toBe("No context added yet"));
  expect(view.queryByTestId("context-panel-clear")).toBeNull();
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});
