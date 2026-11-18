import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

const ran: Array<[string, unknown]> = [];
const realApi = { ...(await import("../../api.js")) }; // a copy: `mock.module` patches a loaded module in place
mock.module("../../api.js", () => ({
  ...realApi,
  runCommand: async (name: string, input: unknown) => {
    ran.push([name, input]);
    return { audit_id: 1, inverse: null, result: {} };
  },
}));
const { addToContext, setSelectedThread } = await import("../../contextBasket.js");
const { ContextNotePrompt } = await import("./ContextNotePrompt.js");

afterEach(() => {
  cleanup();
  ran.length = 0;
  setSelectedThread(null);
});

// Every Add to Context offers a note before it adds: the prompt opens with
// its field focused, and Cmd/Ctrl+Enter adds the item with the note.
test("an add opens the note prompt, and Cmd+Enter adds with the note", async () => {
  setSelectedThread("thr7");
  const view = render(<ContextNotePrompt />);
  expect(view.queryByTestId("context-note-prompt")).toBeNull();
  act(() => void addToContext({ uri: "oxplow:oxplow/file?path=a.rs#lines=1-2", text: "fn a() {}" }));
  const field = await waitFor(() => view.getByTestId("context-note-field") as HTMLTextAreaElement);
  expect(document.activeElement).toBe(field);
  expect(ran).toEqual([]);
  fireEvent.change(field, { target: { value: "Handle the empty case." } });
  fireEvent.keyDown(field, { key: "Enter", metaKey: true });
  await waitFor(() =>
    expect(ran).toEqual([
      ["oxplow.context.add", { thread: "thr7", uri: "oxplow:oxplow/file?path=a.rs#lines=1-2", text: "fn a() {}", note: "Handle the empty case." }],
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

// Without a thread there is nowhere to add: no prompt opens.
test("without a thread no prompt opens", async () => {
  const view = render(<ContextNotePrompt />);
  act(() => void addToContext({ uri: "oxplow:oxplow/page?page=tasks" }));
  expect(view.queryByTestId("context-note-prompt")).toBeNull();
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});
