import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { SqlQueryResult } from "../../tauri-bridge/generated/bindings.js";

const ran: Array<[string, unknown]> = [];
let basketRows: unknown[][] = [];
const realApi = { ...(await import("../../api.js")) }; // a copy: `mock.module` patches a loaded module in place
mock.module("../../api.js", () => ({
  ...realApi,
  querySql: async (sql: string) =>
    ({
      columns: sql.includes("v_context_basket") ? ["id", "name", "next_thread_id"] : ["id", "kind", "source_uri", "label", "text", "note", "basket"],
      rows: sql.includes("v_context_basket") ? basketRows : [],
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
const { dockStore } = await import("../Dock/dockStore.js");
const { resetAppRememberedStore } = await import("../../prefs/remembered.js");

beforeEach(() => {
  basketRows = [];
  resetAppRememberedStore();
});

afterEach(() => {
  cleanup();
  ran.length = 0;
  cancelAddToContext();
  setSelectedThread(null);
  dockStore.close();
});

const panel = () => render(<ContextDockBody streamId={null} onOpenPage={() => {}} />);
const chip = (view: ReturnType<typeof render>, value: string) =>
  view.getAllByTestId("context-add-to").find((c) => c.dataset.value === value) as HTMLButtonElement;
const picked = (view: ReturnType<typeof render>) => view.getAllByTestId("context-add-to").find((c) => c.getAttribute("aria-checked") === "true")?.dataset.value;

// Add to Context opens the Context dock panel with the add card on top: its
// note focused, To at This Thread, and Cmd/Ctrl+Enter adds the item with
// the note, collected in the window's stream.
test("an add opens the Context panel's card, and Cmd+Enter adds to This Thread with the note", async () => {
  setSelectedThread("thr7", "str2");
  const view = panel();
  expect(view.queryByTestId("context-add-card")).toBeNull();
  act(() => void addToContext({ uri: "oxplow:oxplow/file?path=a.rs#lines=1-2", text: "fn a() {}" }));
  expect(dockStore.get().open).toBe("context");
  const field = await waitFor(() => view.getByTestId("context-note-field") as HTMLTextAreaElement);
  await waitFor(() => expect(document.activeElement).toBe(field));
  // A file is named by its path and lines, never its URI.
  expect(view.getByTestId("context-add-title").textContent).toBe("a.rs:1-2");
  await waitFor(() => expect(picked(view)).toBe("@thread"));
  fireEvent.change(field, { target: { value: "Handle the empty case." } });
  fireEvent.keyDown(field, { key: "Enter", metaKey: true });
  await waitFor(() =>
    expect(ran).toEqual([
      ["oxplow.context.add", { thread: "thr7", stream: "str2", uri: "oxplow:oxplow/file?path=a.rs#lines=1-2", text: "fn a() {}", note: "Handle the empty case." }],
    ]),
  );
  expect(view.queryByTestId("context-add-card")).toBeNull();
});

// The note is optional: Add with nothing typed adds the item alone, and
// plain Enter is a new line in the note, not an add.
test("Add with no note adds the item alone; Enter is a new line", async () => {
  setSelectedThread("thr7");
  const view = panel();
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=tasks", label: "Tasks" }));
  const field = await waitFor(() => view.getByTestId("context-note-field"));
  fireEvent.keyDown(field, { key: "Enter" });
  expect(ran).toEqual([]);
  fireEvent.click(view.getByTestId("context-note-add"));
  await waitFor(() => expect(ran).toEqual([["oxplow.context.add", { thread: "thr7", uri: "oxplow:oxplow/page?page=tasks", label: "Tasks" }]]));
});

// Escape drops the add and keeps the panel: it is not passed on.
test("Escape adds nothing", async () => {
  setSelectedThread("thr7");
  const view = panel();
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=tasks" }));
  const field = await waitFor(() => view.getByTestId("context-note-field"));
  fireEvent.keyDown(field, { key: "Escape" });
  await waitFor(() => expect(view.queryByTestId("context-add-card")).toBeNull());
  expect(ran).toEqual([]);
});

// The draft outlives the panel: closing the dock and opening it again
// shows the card with the note typed so far.
test("the note typed so far is kept when the panel closes", async () => {
  setSelectedThread("thr7");
  const view = panel();
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=tasks" }));
  fireEvent.change(await waitFor(() => view.getByTestId("context-note-field")), { target: { value: "Half a thought" } });
  view.unmount();
  const again = panel();
  expect((again.getByTestId("context-note-field") as HTMLTextAreaElement).value).toBe("Half a thought");
});

// A named basket is picked from To; the next add starts at it.
test("an add goes to the basket picked, and the next starts there", async () => {
  setSelectedThread("thr7");
  basketRows = [[1, "Auth", null]];
  const view = panel();
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=tasks" }));
  await waitFor(() => expect(chip(view, "Auth")).toBeTruthy());
  fireEvent.click(chip(view, "Auth"));
  fireEvent.click(view.getByTestId("context-note-add"));
  await waitFor(() => expect(ran).toEqual([["oxplow.context.add", { basket: "Auth", uri: "oxplow:oxplow/page?page=tasks" }]]));
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=problems" }));
  await waitFor(() => expect(picked(view)).toBe("Auth"));
});

// + New basket asks for a name; Enter in it adds to the new basket, and
// Escape goes back to the choices.
test("+ New basket takes a name and adds to it", async () => {
  setSelectedThread("thr7");
  const view = panel();
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=tasks" }));
  await waitFor(() => expect(picked(view)).toBe("@thread"));
  fireEvent.click(chip(view, "@new"));
  const name = await waitFor(() => view.getByTestId("context-note-new-basket") as HTMLInputElement);
  await waitFor(() => expect(document.activeElement).toBe(name));
  fireEvent.keyDown(name, { key: "Escape" });
  expect(picked(view)).toBe("@thread");
  expect(view.getByTestId("context-add-card")).toBeTruthy();
  fireEvent.click(chip(view, "@new"));
  const again = await waitFor(() => view.getByTestId("context-note-new-basket") as HTMLInputElement);
  expect((view.getByTestId("context-note-add") as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(again, { target: { value: "  Release notes " } });
  fireEvent.keyDown(again, { key: "Enter" });
  await waitFor(() => expect(ran).toEqual([["oxplow.context.add", { basket: "Release notes", uri: "oxplow:oxplow/page?page=tasks" }]]));
});

// With no thread selected the card still opens: This Thread can't be
// picked, so it starts at a named basket, or at a new one with none.
test("without a thread the card starts at a named basket or a new one", async () => {
  basketRows = [[1, "Auth", null]];
  const view = panel();
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=tasks" }));
  await waitFor(() => expect(picked(view)).toBe("Auth"));
  expect(chip(view, "@thread").disabled).toBe(true);
  cleanup();
  cancelAddToContext();
  basketRows = [];
  const empty = panel();
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=tasks" }));
  await waitFor(() => empty.getByTestId("context-note-new-basket"));
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});
