import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render as rtlRender, waitFor } from "@testing-library/react";
import { useState, type ReactElement, type ReactNode } from "react";

import type { PageAction, VizRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { VizActionRunnerContext, type VizActionRunner } from "./vizActionRunner.js";
import { PageNavigationContext, type PageNavigation, type TabEditing } from "../tabs/PageNavigationContext.js";
import { PageCommentsContext } from "./pageComments.js";

// A markdown viz with an edit action is an editor whose Save is that
// action: the draft is the tab's, Save only when changed, Discard puts
// the saved text back, a re-read never wipes a draft.

const realApi = { ...(await import("../api.js")) }; // a copy: `mock.module` patches a loaded module in place
const saves: { action: string; row: unknown; input: unknown }[] = [];
const targets: string[] = [];
mock.module("../api.js", () => ({
  ...realApi,
  listCommentsForTarget: async (target: string) => {
    targets.push(target);
    return [];
  },
  subscribeCommentEvents: () => () => {},
  listExtensions: async () => [],
}));
const { VizView } = await import("./VizView.js");

// A viz presses through the page that draws it: a runner stands in for it.
const press: VizActionRunner = async (_id, action, _p, row, _s, _c, input) => {
  saves.push({ action, row, input });
  return { result: null, audit_id: 1, event_id: null, inverse: null };
};
const wrapper = ({ children }: { children: ReactNode }) => <VizActionRunnerContext.Provider value={press}>{children}</VizActionRunnerContext.Provider>;
const render = (ui: ReactElement) => rtlRender(ui, { wrapper });

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
} as unknown as PageAction;

const runOf = (body: string, actions: PageAction[] = [save]): VizRun =>
  ({
    config: {
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
    result: { columns: ["slug", "body"], rows: [["notes", body] as SqlCell[]], truncated: false, reads: { datasets: [], tables: [], measures: [] } },
  }) as unknown as VizRun;

/** A tab whose history state keeps the draft. */
function Tab({ state, children, setEditing }: { state: Record<string, unknown>; children: ReactNode; setEditing?: PageNavigation["setEditing"] }) {
  const [held, setHeld] = useState(state);
  const nav = {
    setEditing,
    historyState: held,
    setHistoryState: (patch: Record<string, unknown>) => {
      Object.assign(state, patch);
      setHeld({ ...state });
    },
  } as unknown as PageNavigation;
  return <PageNavigationContext.Provider value={nav}>{children}</PageNavigationContext.Provider>;
}

type Editable = HTMLElement & { editor?: { commands: { insertContentAt(pos: number, text: string): boolean } } };
const editorOf = (view: ReturnType<typeof render>) => waitFor(() => view.getByTestId("viz-markdown-editor") as Editable);
const typeAt = async (el: Editable, text: string) =>
  act(async () => {
    el.editor?.commands.insertContentAt(1, text);
  });

test("without an edit action it is the viewer", async () => {
  const view = render(<VizView run={runOf("# Hello", [])} onOpenPage={() => {}} />);
  await waitFor(() => expect(view.getByTestId("viz-markdown").querySelector("h1")?.textContent).toBe("Hello"));
  expect(view.queryByTestId("viz-markdown-save")).toBeNull();
});

test("typing is a draft in the tab; Save runs the edit action with the text and clears it", async () => {
  const state: Record<string, unknown> = {};
  const view = render(
    <Tab state={state}>
      <VizView run={runOf("Old text.")} onOpenPage={() => {}} />
    </Tab>,
  );
  const editor = await editorOf(view);
  const saveButton = view.getByTestId("viz-markdown-save") as HTMLButtonElement;
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
      <VizView run={runOf("Old text.")} onOpenPage={() => {}} />
    </Tab>,
  );
  const editor = await editorOf(view);
  await typeAt(editor, "Typed. ");
  fireEvent.click(view.getByTestId("viz-markdown-discard"));
  expect(state["markdown:x/page"]).toBeNull();
  await waitFor(() => expect(editor.textContent).toBe("Old text."));
  expect((view.getByTestId("viz-markdown-save") as HTMLButtonElement).disabled).toBe(true);
});

test("a re-read keeps a draft; a clean editor follows the saved text", async () => {
  const state: Record<string, unknown> = { "markdown:x/page": "My draft." };
  const view = render(
    <Tab state={state}>
      <VizView run={runOf("Saved one.")} onOpenPage={() => {}} />
    </Tab>,
  );
  const editor = await editorOf(view);
  await waitFor(() => expect(editor.textContent).toBe("My draft."));
  view.rerender(
    <Tab state={state}>
      <VizView run={runOf("Saved two.")} onOpenPage={() => {}} />
    </Tab>,
  );
  await act(async () => new Promise((r) => setTimeout(r, 0)));
  expect(editor.textContent).toBe("My draft.");

  cleanup();
  const clean: Record<string, unknown> = {};
  const again = render(
    <Tab state={clean}>
      <VizView run={runOf("Saved one.")} onOpenPage={() => {}} />
    </Tab>,
  );
  const second = await editorOf(again);
  again.rerender(
    <Tab state={clean}>
      <VizView run={runOf("Saved two.")} onOpenPage={() => {}} />
    </Tab>,
  );
  await waitFor(() => expect(second.textContent).toBe("Saved two."));
});

test("on a page that takes comments, the editor's comments are on the page's URI", async () => {
  const view = render(
    <PageCommentsContext.Provider value={{ target: "oxplow:oxplow/wiki?slug=notes", streamId: "str1", threadId: null }}>
      <VizView run={runOf("Some text.")} onOpenPage={() => {}} />
    </PageCommentsContext.Provider>,
  );
  await editorOf(view);
  await waitFor(() => expect(targets).toContain("oxplow:oxplow/wiki?slug=notes"));
});

test("a draft makes its tab dirty, and the tab's Save saves it", async () => {
  const state: Record<string, unknown> = {};
  let editing: TabEditing | null = null;
  const setEditing = (e: TabEditing | null) => {
    editing = e;
  };
  const view = render(
    <Tab state={state} setEditing={setEditing}>
      <VizView run={runOf("Old text.")} onOpenPage={() => {}} />
    </Tab>,
  );
  const editor = await editorOf(view);
  await waitFor(() => expect(editing?.dirty).toBe(false));
  await typeAt(editor, "New. ");
  await waitFor(() => expect(editing?.dirty).toBe(true));
  await act(async () => {
    editing!.save();
  });
  await waitFor(() => expect(saves).toEqual([{ action: "save", row: { slug: "notes", body: "Old text." }, input: { body: "New. Old text." } }]));
});

// A document that is the page's own file links to its siblings the way it
// is read: with the page that draws it, when that page was chosen (the
// default is the absent key). A document shown inside another page (a
// README on a commit page) carries nothing.
const document = (base: string): VizRun =>
  ({
    config: {
      id: "x/doc",
      extension: "x",
      slug: "doc",
      title: "Doc",
      viz: "markdown",
      markdown: { text: "text", base: "uri" },
      params: [],
      columns: [],
      actions: [],
      parts: [],
      origin: { extension: "", source: null, running: "", deployment: null, bundled: false },
    },
    params: {},
    result: { columns: ["text", "uri"], rows: [["See [DEV](DEV.md) and [main](src/main.rs).", base] as SqlCell[]], truncated: false, reads: { datasets: [], tables: [], measures: [] } },
  }) as unknown as VizRun;

const renderers = [
  { page: "oxplow/editor", title: "Editor", default: true },
  { page: "oxplow/markdown", title: "Markdown", default: false },
];

function openedFrom(ref: string, viewParams: Record<string, SqlCell>, run: VizRun): string[] {
  const opened: string[] = [];
  const nav = { navigate: () => {}, openRef: (r: string) => opened.push(r), ref: { id: ref }, renderers, viewParams } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <VizView run={run} streamId="str1" />
    </PageNavigationContext.Provider>,
  );
  fireEvent.click(view.container.querySelector('a[href="DEV.md"]')!);
  fireEvent.click(view.container.querySelector('a[href="src/main.rs"]')!);
  return opened;
}

test("a document's link to a sibling of its kind opens with the page that draws it", () => {
  const readme = "oxplow:oxplow/file?path=README.md";
  expect(openedFrom(`${readme}#oxplow.renderer=oxplow/markdown`, { "oxplow.renderer": "oxplow/markdown" }, document(readme))).toEqual([
    "oxplow:oxplow/file?path=DEV.md#oxplow.renderer=oxplow/markdown",
    "oxplow:oxplow/file?path=src/main.rs",
  ]);
  // Drawn by the default page: the link carries no key, as the tab's own address has none.
  expect(openedFrom(readme, {}, document(readme))).toEqual(["oxplow:oxplow/file?path=DEV.md", "oxplow:oxplow/file?path=src/main.rs"]);
  // The document is not the page's own file: its links carry nothing of the page.
  expect(openedFrom("oxplow:oxplow/commit?sha=abc1234#oxplow.renderer=x/commit", { "oxplow.renderer": "x/commit" }, document(readme))).toEqual([
    "oxplow:oxplow/file?path=DEV.md",
    "oxplow:oxplow/file?path=src/main.rs",
  ]);
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../api.js", () => realApi);
});
