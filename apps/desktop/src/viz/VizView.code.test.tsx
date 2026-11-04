import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { VizConfig, VizRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";

// `code` shows each row's file at its revision, read-only, its lines
// marked; `diff` one file between two revisions. Both read the files
// themselves and open the working file on Open file.

const reads: { path: string; revision: string | undefined; filesystem?: string }[] = [];
const models: string[] = [];
const decorations: { startLineNumber: number; endLineNumber: number }[] = [];
const diffModels: number[] = [];
const actions: string[] = [];
const commentTargets: string[] = [];

function fakeEditor() {
  let model: { lines: number } | null = null;
  return {
    setModel(m: { lines: number } | { original: unknown; modified: unknown } | null) {
      if (m && "original" in m) diffModels.push(1);
      model = m && "lines" in m ? m : null;
    },
    getModel: () => model,
    dispose() {},
    deltaDecorations(_: unknown, ds: { range: { startLineNumber: number; endLineNumber: number } }[]) {
      decorations.push(...ds.map((d) => d.range));
      return [];
    },
    setPosition() {},
    revealLineNearTop() {},
    getTopForLineNumber: (n: number) => n * 20,
    getOption: () => 20,
    onDidScrollChange: () => ({ dispose() {} }),
    onDidChangeCursorPosition: () => ({ dispose() {} }),
    onMouseDown: () => ({ dispose() {} }),
    addAction(a: { id: string }) {
      actions.push(a.id);
      return { dispose() {} };
    },
    saveViewState: () => null,
    setScrollTop() {},
    setScrollLeft() {},
    getModifiedEditor() {
      return { ...fakeEditor(), createContextKey: () => ({ set() {} }), addAction() {}, getPosition: () => null };
    },
    getLineChanges: () => [],
    onDidUpdateDiff: () => ({ dispose() {} }),
  };
}

const realMonaco = { ...(await import("monaco-editor")) }; // a copy: `mock.module` patches a loaded module in place
mock.module("monaco-editor", () => ({
  editor: {
    create: () => fakeEditor(),
    createDiffEditor: () => fakeEditor(),
    createModel: (text: string) => (models.push(text), { lines: text.split("\n").length, getLineCount: () => text.split("\n").length, getLineMaxColumn: () => 1, getValue: () => text, dispose() {} }),
    EditorOption: { lineHeight: 0 },
  },
  Range: class {
    constructor(
      public startLineNumber: number,
      _a: number,
      public endLineNumber: number,
    ) {}
  },
}));

const realApi = { ...(await import("../api.js")) }; // a copy: `mock.module` patches a loaded module in place
mock.module("../api.js", () => ({
  ...realApi,
  statFile: async (_stream: string, path: string) => {
    if (path === "gone.rs") return null;
    if (path === "logo.png") return { kind: "file", size: 2048, modifiedAt: null, isText: false, language: null };
    if (path === "huge.log") return { kind: "file", size: 9_000_000, modifiedAt: null, isText: true, language: null };
    return { kind: "file", size: 100, modifiedAt: null, isText: true, language: null };
  },
  readAt: async (_stream: string, path: string, at: { rev?: string; filesystem?: string }) => {
    reads.push({ path, revision: at.rev, ...(at.filesystem ? { filesystem: at.filesystem } : {}) });
    return Array.from({ length: 50 }, (_, i) => `line ${i + 1}`).join("\n");
  },
  listCommentsForTarget: async (target: string) => {
    commentTargets.push(target);
    return [];
  },
  subscribeCommentEvents: () => () => {},
}));
const { PageCommentsContext } = await import("./pageComments.js");

const { VizView } = await import("./VizView.js");

afterEach(() => {
  cleanup();
  reads.length = 0;
  models.length = 0;
  decorations.length = 0;
  diffModels.length = 0;
  actions.length = 0;
  commentTargets.length = 0;
});

// On a page that takes comments, a code viz's file takes them on its URI
// at its revision (Monaco's menu has Add Comment); elsewhere it doesn't.
test("a code viz's file takes comments on its URI at its revision, on a page that takes them", async () => {
  const vizRun = run({ viz: "code", code: { path: "p", revision: "r", lines: null, sync: false } }, ["p", "r"], [["src/a.rs", "git:abc1234"]]);
  const plain = render(<VizView run={vizRun} streamId="str1" onOpenPage={() => {}} />);
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(actions).toEqual([]);
  expect(commentTargets).toEqual([]);
  plain.unmount();
  render(
    <PageCommentsContext.Provider value={{ target: "oxplow:oxplow/file?path=src%2Fa.rs&rev=git%3Aabc1234", streamId: "str1", threadId: null }}>
      <VizView run={vizRun} streamId="str1" onOpenPage={() => {}} />
    </PageCommentsContext.Provider>,
  );
  await waitFor(() => expect(actions).toEqual(["oxplow.comment.add"]));
  const { fileUri } = await import("../uri/uri.js");
  await waitFor(() => expect(commentTargets).toEqual([fileUri("src/a.rs", "git:abc1234")]));
});

const base = {
  id: "x/c",
  extension: "x",
  slug: "c",
  title: "C",
  description: "",
  query: "",
  params: [],
  columns: [],
  empty: null,
  chart: null,
  tree: null,
  tile: null,
  timeline: null,
  steps: null,
  hunks: null,
  code: null,
  media: null,
  diff: null,
  markdown: null,
  form: null,
  custom: null,
  group: null,
  emphasis: null,
  depth: null,
  parts: [],
  launcherCategory: null,
  hidden: false,
  actions: [],
  alert: null,
  needs: [],
  origin: { extension: "x", source: null, running: "", deployment: null, bundled: false },
  tool: null,
};

function run(viz: Partial<VizConfig>, columns: string[], rows: SqlCell[][]): VizRun {
  return {
    config: { ...base, ...viz },
    params: {},
    result: { columns, rows, truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} },
  } as unknown as VizRun;
}

function nav(navigated: TabRef[]): PageNavigation {
  return { navigate: (to) => navigated.push(to), goBack() {}, goForward() {}, canGoBack: false, canGoForward: false };
}

test("a code viz shows each row's file at its revision, its lines marked, and opens the working file", async () => {
  const navigated: TabRef[] = [];
  const view = render(
    <PageNavigationContext.Provider value={nav(navigated)}>
      <VizView
        run={run({ viz: "code", code: { path: "p", revision: "r", lines: "l", sync: true } }, ["p", "r", "l"], [
          ["src/a.rs", "git:abc1234", "12-14"],
          ["gone.rs", "working", null],
        ])}
        streamId="str1"
        onOpenPage={() => {}}
      />
    </PageNavigationContext.Provider>,
  );
  // `gone.rs` is `stat`ed first and found missing, so it is never read.
  await waitFor(() => expect(reads).toHaveLength(1));
  const files = view.getAllByTestId("viz-code-file");
  expect(files.map((f) => f.querySelector("[data-testid=viz-code-path]")!.textContent)).toEqual(["src/a.rs", "gone.rs"]);
  expect(files[0]!.textContent).toContain(":12-14");
  expect(files[0]!.textContent).toContain("@abc1234");
  expect(reads).toEqual([{ path: "src/a.rs", revision: "git:abc1234" }]);
  await waitFor(() => expect(decorations).toEqual([expect.objectContaining({ startLineNumber: 12, endLineNumber: 14 })]));
  await waitFor(() => expect(view.getByTestId("viz-code-problem").textContent).toBe("Not there at working tree"));
  fireEvent.click(files[0]!.querySelector("[data-testid=viz-code-open]")!);
  expect(navigated.map((r) => r.id)).toEqual(["oxplow:oxplow/file?path=src/a.rs"]);
});

// A code viz that is the page itself (the Text view) draws no Open file;
// a snippet in another page does.
test("a code viz that is the page's own view has no Open file", async () => {
  const vizRun = run({ viz: "code", code: { path: "p", revision: "r", lines: null, sync: false } }, ["p", "r"], [["src/a.rs", "git:abc1234"]]);
  const snippet = render(<VizView run={vizRun} streamId="str1" onOpenPage={() => {}} />);
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(snippet.queryByTestId("viz-code-open")).not.toBeNull();
  snippet.unmount();
  const page = render(<VizView run={vizRun} streamId="str1" onOpenPage={() => {}} isPage />);
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(page.queryByTestId("viz-code-open")).toBeNull();
});

test("a diff viz shows its first row's file between its two revisions", async () => {
  const view = render(
    <VizView
      run={run({ viz: "diff", diff: { path: "p", from: "a", to: "b", lines: null } }, ["p", "a", "b"], [
        ["src/a.rs", "git:abc1234", "working"],
        ["src/b.rs", "git:abc1234", "working"],
      ])}
      streamId={null}
      onOpenPage={() => {}}
    />,
  );
  await waitFor(() => expect(diffModels).toHaveLength(1));
  expect(view.getByTestId("viz-diff").textContent).toContain("src/a.rs");
  expect(reads).toEqual([
    { path: "src/a.rs", revision: "git:abc1234" },
    { path: "src/a.rs", revision: "working" },
  ]);
});

test("a binary or too-large file is not read; the placeholder says what it is", async () => {
  render(
    <VizView
      run={run({ viz: "code", code: { path: "p", revision: "r", lines: null, sync: false } }, ["p", "r"], [
        ["logo.png", "working"],
        ["huge.log", "working"],
      ])}
      streamId="str1"
      onOpenPage={() => {}}
    />,
  );
  await waitFor(() => expect(models).toHaveLength(2));
  expect(models).toEqual(["(binary file, 2048 bytes)", "(too large to show, 9000000 bytes)"]);
  expect(reads).toEqual([]);
});

test("a code viz row names its filesystem; with no revision it reads that filesystem's current content", async () => {
  const view = render(
    <VizView
      run={run({ viz: "code", code: { path: "p", filesystem: "f", revision: "r", lines: null, sync: false } }, ["p", "f", "r"], [
        ["acme/page.yaml", "oxplow:extensions", null],
      ])}
      streamId="str1"
      onOpenPage={() => {}}
    />,
  );
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(reads).toEqual([{ path: "acme/page.yaml", revision: undefined, filesystem: "oxplow:extensions" }]);
  expect(view.getByTestId("viz-code-file").textContent).toContain("@oxplow:extensions");
});

// `mock.module` outlives this file: put the real modules back for the files that run after it.
afterAll(() => {
  mock.module("../api.js", () => realApi);
  mock.module("monaco-editor", () => realMonaco);
});
