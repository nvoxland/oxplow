import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { useState, type ReactNode } from "react";

import type { LensAction, LensRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import { PageCommentsContext } from "./pageComments.js";

// A markdown lens with an edit action is an editor whose Save is that
// action: the draft is the tab's, Save only when changed, Discard puts
// the saved text back, a re-read never wipes a draft.

const realApi = await import("../api.js");
const saves: { action: string; row: unknown; input: unknown }[] = [];
const targets: string[] = [];
mock.module("../api.js", () => ({
  ...realApi,
  runLensAction: async (_id: string, action: string, _p: unknown, row: unknown, _s: unknown, _c: boolean, input: unknown) => {
    saves.push({ action, row, input });
    return { result: null, audit_id: 1, event_id: null, inverse: null };
  },
  listCommentsForTarget: async (target: string) => {
    targets.push(target);
    return [];
  },
  subscribeCommentEvents: () => () => {},
  listExtensions: async () => [],
}));
const { LensResultView } = await import("./LensResultView.js");

afterEach(() => {
  cleanup();
  saves.length = 0;
  targets.length = 0;
});

const save = {
  id: "save",
  label: "Save",
  command: "oxplow.knowledge.write_page",
  input: { slug: "{{row.slug}}", body: "{{row.body}}" },
  row: true,
  group: null,
  form: true,
  fixed: ["slug"],
  choices: {},
  then: "open",
  when: null,
  primary: false,
  drag: false,
  bulk: false,
  key: null,
  edit: true,
  editField: "body",
} as unknown as LensAction;

const runOf = (body: string, actions: LensAction[] = [save]): LensRun =>
  ({
    lens: {
      id: "x/page",
      extension: "x",
      slug: "page",
      title: "Page",
      viz: "markdown",
      markdown: { text: "body" },
      params: [],
      columns: [],
      actions,
      parts: [],
      origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
    },
    params: {},
    result: { columns: ["slug", "body"], rows: [["notes", body] as SqlCell[]], truncated: false, reads: { models: [], tables: [], measures: [] } },
  }) as unknown as LensRun;

/** A tab whose history state keeps the draft. */
function Tab({ state, children }: { state: Record<string, unknown>; children: ReactNode }) {
  const [held, setHeld] = useState(state);
  const nav = {
    historyState: held,
    setHistoryState: (patch: Record<string, unknown>) => {
      Object.assign(state, patch);
      setHeld({ ...state });
    },
  } as unknown as PageNavigation;
  return <PageNavigationContext.Provider value={nav}>{children}</PageNavigationContext.Provider>;
}

type Editable = HTMLElement & { editor?: { commands: { insertContentAt(pos: number, text: string): boolean } } };
const editorOf = (view: ReturnType<typeof render>) => waitFor(() => view.getByTestId("lens-markdown-editor") as Editable);
const typeAt = async (el: Editable, text: string) =>
  act(async () => {
    el.editor?.commands.insertContentAt(1, text);
  });

test("without an edit action it is the viewer", async () => {
  const view = render(<LensResultView run={runOf("# Hello", [])} onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getByTestId("lens-markdown").querySelector("h1")?.textContent).toBe("Hello"));
  expect(view.queryByTestId("lens-markdown-save")).toBeNull();
});

test("typing is a draft in the tab; Save runs the edit action with the text and clears it", async () => {
  const state: Record<string, unknown> = {};
  const view = render(
    <Tab state={state}>
      <LensResultView run={runOf("Old text.")} onOpenPage={() => {}} />
    </Tab>,
  );
  const editor = await editorOf(view);
  const saveButton = view.getByTestId("lens-markdown-save") as HTMLButtonElement;
  expect(saveButton.disabled).toBe(true);
  await typeAt(editor, "New. ");
  expect(state["markdown:x/page"]).toBe("New. Old text.");
  expect(saveButton.disabled).toBe(false);
  await act(async () => {
    fireEvent.click(saveButton);
  });
  await waitFor(() => expect(saves).toEqual([{ action: "save", row: { slug: "notes", body: "Old text." }, input: { body: "New. Old text." } }]));
  await waitFor(() => expect(state["markdown:x/page"]).toBeNull());
});

test("Discard puts the saved text back", async () => {
  const state: Record<string, unknown> = {};
  const view = render(
    <Tab state={state}>
      <LensResultView run={runOf("Old text.")} onOpenPage={() => {}} />
    </Tab>,
  );
  const editor = await editorOf(view);
  await typeAt(editor, "Typed. ");
  fireEvent.click(view.getByTestId("lens-markdown-discard"));
  expect(state["markdown:x/page"]).toBeNull();
  await waitFor(() => expect(editor.textContent).toBe("Old text."));
  expect((view.getByTestId("lens-markdown-save") as HTMLButtonElement).disabled).toBe(true);
});

test("a re-read keeps a draft; a clean editor follows the saved text", async () => {
  const state: Record<string, unknown> = { "markdown:x/page": "My draft." };
  const view = render(
    <Tab state={state}>
      <LensResultView run={runOf("Saved one.")} onOpenPage={() => {}} />
    </Tab>,
  );
  const editor = await editorOf(view);
  await waitFor(() => expect(editor.textContent).toBe("My draft."));
  view.rerender(
    <Tab state={state}>
      <LensResultView run={runOf("Saved two.")} onOpenPage={() => {}} />
    </Tab>,
  );
  await act(async () => new Promise((r) => setTimeout(r, 0)));
  expect(editor.textContent).toBe("My draft.");

  cleanup();
  const clean: Record<string, unknown> = {};
  const again = render(
    <Tab state={clean}>
      <LensResultView run={runOf("Saved one.")} onOpenPage={() => {}} />
    </Tab>,
  );
  const second = await editorOf(again);
  again.rerender(
    <Tab state={clean}>
      <LensResultView run={runOf("Saved two.")} onOpenPage={() => {}} />
    </Tab>,
  );
  await waitFor(() => expect(second.textContent).toBe("Saved two."));
});

test("on a page that takes comments, the editor's comments are on the page's URI", async () => {
  const view = render(
    <PageCommentsContext.Provider value={{ target: "oxplow:oxplow/wiki?slug=notes", streamId: "str1", threadId: null }}>
      <LensResultView run={runOf("Some text.")} onOpenPage={() => {}} />
    </PageCommentsContext.Provider>,
  );
  await editorOf(view);
  await waitFor(() => expect(targets).toContain("oxplow:oxplow/wiki?slug=notes"));
});
