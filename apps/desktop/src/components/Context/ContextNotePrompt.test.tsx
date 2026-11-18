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
      columns: sql.includes("v_context_basket") ? ["id", "name", "attached_thread_id"] : ["id", "kind", "source_uri", "label", "text", "note", "basket"],
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
const { addToContext, setSelectedThread } = await import("../../contextBasket.js");
const { ContextNotePrompt } = await import("./ContextNotePrompt.js");
const { resetAppRememberedStore } = await import("../../prefs/remembered.js");

beforeEach(() => {
  basketRows = [];
  resetAppRememberedStore();
});

afterEach(() => {
  cleanup();
  ran.length = 0;
  setSelectedThread(null);
});

const picker = (view: ReturnType<typeof render>) => view.getByTestId("context-note-basket") as HTMLSelectElement;

// Every Add to Context offers a note before it adds: the prompt opens with
// its field focused, To at This Thread, and Cmd/Ctrl+Enter adds the item
// with the note, collected in the window's stream.
test("an add opens the prompt, and Cmd+Enter adds to This Thread with the note", async () => {
  setSelectedThread("thr7", "str2");
  const view = render(<ContextNotePrompt />);
  expect(view.queryByTestId("context-note-prompt")).toBeNull();
  act(() => void addToContext({ uri: "oxplow:oxplow/file?path=a.rs#lines=1-2", text: "fn a() {}" }));
  const field = await waitFor(() => view.getByTestId("context-note-field") as HTMLTextAreaElement);
  expect(document.activeElement).toBe(field);
  expect(picker(view).value).toBe("@thread");
  expect(ran).toEqual([]);
  fireEvent.change(field, { target: { value: "Handle the empty case." } });
  fireEvent.keyDown(field, { key: "Enter", metaKey: true });
  await waitFor(() =>
    expect(ran).toEqual([
      ["oxplow.context.add", { thread: "thr7", stream: "str2", uri: "oxplow:oxplow/file?path=a.rs#lines=1-2", text: "fn a() {}", note: "Handle the empty case." }],
    ]),
  );
  expect(view.queryByTestId("context-note-prompt")).toBeNull();
});

// The note is optional: Add with nothing typed adds the item alone, and
// plain Enter is a new line in the note, not an add.
test("Add with no note adds the item alone; Enter is a new line", async () => {
  setSelectedThread("thr7");
  const view = render(<ContextNotePrompt />);
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=tasks", label: "Tasks" }));
  const field = await waitFor(() => view.getByTestId("context-note-field"));
  fireEvent.keyDown(field, { key: "Enter" });
  expect(ran).toEqual([]);
  fireEvent.click(view.getByTestId("context-note-add"));
  await waitFor(() => expect(ran).toEqual([["oxplow.context.add", { thread: "thr7", uri: "oxplow:oxplow/page?page=tasks", label: "Tasks" }]]));
});

// Escape closes the prompt and adds nothing.
test("Escape adds nothing", async () => {
  setSelectedThread("thr7");
  const view = render(<ContextNotePrompt />);
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=tasks" }));
  const field = await waitFor(() => view.getByTestId("context-note-field"));
  fireEvent.keyDown(field, { key: "Escape" });
  await waitFor(() => expect(view.queryByTestId("context-note-prompt")).toBeNull());
  expect(ran).toEqual([]);
});

// A named basket is picked from To; the next add starts at it.
test("an add goes to the basket picked, and the next starts there", async () => {
  setSelectedThread("thr7");
  basketRows = [[1, "Auth", null]];
  const view = render(<ContextNotePrompt />);
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=tasks" }));
  await waitFor(() => expect(view.getByText("Auth")).toBeTruthy());
  fireEvent.change(picker(view), { target: { value: "Auth" } });
  fireEvent.click(view.getByTestId("context-note-add"));
  await waitFor(() => expect(ran).toEqual([["oxplow.context.add", { basket: "Auth", uri: "oxplow:oxplow/page?page=tasks" }]]));
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=problems" }));
  await waitFor(() => expect(picker(view).value).toBe("Auth"));
});

// New basket… asks for a name; Enter in it adds to the new basket, and
// Escape goes back to the picker.
test("New basket… takes a name and adds to it", async () => {
  setSelectedThread("thr7");
  const view = render(<ContextNotePrompt />);
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=tasks" }));
  await waitFor(() => view.getByTestId("context-note-basket"));
  fireEvent.change(picker(view), { target: { value: "@new" } });
  const name = await waitFor(() => view.getByTestId("context-note-new-basket") as HTMLInputElement);
  expect(document.activeElement).toBe(name);
  fireEvent.keyDown(name, { key: "Escape" });
  expect(picker(view).value).toBe("@thread");
  expect(view.getByTestId("context-note-prompt")).toBeTruthy();
  fireEvent.change(picker(view), { target: { value: "@new" } });
  const again = await waitFor(() => view.getByTestId("context-note-new-basket") as HTMLInputElement);
  expect((view.getByTestId("context-note-add") as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(again, { target: { value: "  Release notes " } });
  fireEvent.keyDown(again, { key: "Enter" });
  await waitFor(() => expect(ran).toEqual([["oxplow.context.add", { basket: "Release notes", uri: "oxplow:oxplow/page?page=tasks" }]]));
});

// With no thread selected the prompt still opens: This Thread can't be
// picked, so it starts at a named basket, or at New basket… with none.
test("without a thread the prompt opens at a named basket or a new one", async () => {
  basketRows = [[1, "Auth", null]];
  const view = render(<ContextNotePrompt />);
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=tasks" }));
  await waitFor(() => expect(picker(view).value).toBe("Auth"));
  const thread = view.getByRole("option", { name: /This Thread/ }) as HTMLOptionElement;
  expect(thread.disabled).toBe(true);
  cleanup();
  basketRows = [];
  const empty = render(<ContextNotePrompt />);
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=tasks" }));
  await waitFor(() => empty.getByTestId("context-note-new-basket"));
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});
