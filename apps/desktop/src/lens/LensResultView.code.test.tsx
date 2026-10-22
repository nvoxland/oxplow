import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Lens, LensRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";

// `code` shows each row's file at its revision, read-only, its lines
// marked; `diff` one file between two revisions. Both read the files
// themselves and open the working file on Open file.

const reads: { path: string; revision: string }[] = [];
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

mock.module("monaco-editor", () => ({
  editor: {
    create: () => fakeEditor(),
    createDiffEditor: () => fakeEditor(),
    createModel: (text: string) => ({ lines: text.split("\n").length, getLineCount: () => text.split("\n").length, getLineMaxColumn: () => 1, getValue: () => text, dispose() {} }),
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

const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  readAt: async (_stream: string, path: string, revision: string) => {
    reads.push({ path, revision });
    return path === "gone.rs" ? null : Array.from({ length: 50 }, (_, i) => `line ${i + 1}`).join("\n");
  },
  listCommentsForTarget: async (target: string) => {
    commentTargets.push(target);
    return [];
  },
  subscribeCommentEvents: () => () => {},
}));
const { PageCommentsContext } = await import("./pageComments.js");

const { LensResultView } = await import("./LensResultView.js");

afterEach(() => {
  cleanup();
  reads.length = 0;
  decorations.length = 0;
  diffModels.length = 0;
  actions.length = 0;
  commentTargets.length = 0;
});

// On a page that takes comments, a code viz's file takes them on its URI
// at its revision (Monaco's menu has Add Comment); elsewhere it doesn't.
test("a code lens's file takes comments on its URI at its revision, on a page that takes them", async () => {
  const lensRun = run({ viz: "code", code: { path: "p", revision: "r", lines: null, sync: false } }, ["p", "r"], [["src/a.rs", "git:abc1234"]]);
  const plain = render(<LensResultView run={lensRun} streamId="str1" onOpenPage={() => {}} />);
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(actions).toEqual([]);
  expect(commentTargets).toEqual([]);
  plain.unmount();
  render(
    <PageCommentsContext.Provider value={{ target: "oxplow:oxplow/file?path=src%2Fa.rs&rev=git%3Aabc1234", streamId: "str1", threadId: null }}>
      <LensResultView run={lensRun} streamId="str1" onOpenPage={() => {}} />
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
  timeline: null,
  steps: null,
  hunks: null,
  code: null,
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

function run(lens: Partial<Lens>, columns: string[], rows: SqlCell[][]): LensRun {
  return {
    lens: { ...base, ...lens },
    params: {},
    result: { columns, rows, truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} },
  } as unknown as LensRun;
}

function nav(navigated: TabRef[]): PageNavigation {
  return { navigate: (to) => navigated.push(to), goBack() {}, goForward() {}, canGoBack: false, canGoForward: false };
}

test("a code lens shows each row's file at its revision, its lines marked, and opens the working file", async () => {
  const navigated: TabRef[] = [];
  const view = render(
    <PageNavigationContext.Provider value={nav(navigated)}>
      <LensResultView
        run={run({ viz: "code", code: { path: "p", revision: "r", lines: "l", sync: true } }, ["p", "r", "l"], [
          ["src/a.rs", "git:abc1234", "12-14"],
          ["gone.rs", "working", null],
        ])}
        streamId="str1"
        onOpenPage={() => {}}
      />
    </PageNavigationContext.Provider>,
  );
  await waitFor(() => expect(reads).toHaveLength(2));
  const files = view.getAllByTestId("lens-code-file");
  expect(files.map((f) => f.querySelector("[data-testid=lens-code-path]")!.textContent)).toEqual(["src/a.rs", "gone.rs"]);
  expect(files[0]!.textContent).toContain(":12-14");
  expect(files[0]!.textContent).toContain("@abc1234");
  expect(reads).toEqual([
    { path: "src/a.rs", revision: "git:abc1234" },
    { path: "gone.rs", revision: "working" },
  ]);
  await waitFor(() => expect(decorations).toEqual([expect.objectContaining({ startLineNumber: 12, endLineNumber: 14 })]));
  await waitFor(() => expect(view.getByTestId("lens-code-problem").textContent).toBe("Not there at working tree"));
  fireEvent.click(files[0]!.querySelector("[data-testid=lens-code-open]")!);
  expect(navigated.map((r) => r.id)).toEqual(["oxplow:oxplow/file?path=src/a.rs"]);
});

test("a diff lens shows its first row's file between its two revisions", async () => {
  const view = render(
    <LensResultView
      run={run({ viz: "diff", diff: { path: "p", from: "a", to: "b", lines: null } }, ["p", "a", "b"], [
        ["src/a.rs", "git:abc1234", "working"],
        ["src/b.rs", "git:abc1234", "working"],
      ])}
      streamId={null}
      onOpenPage={() => {}}
    />,
  );
  await waitFor(() => expect(diffModels).toHaveLength(1));
  expect(view.getByTestId("lens-diff").textContent).toContain("src/a.rs");
  expect(reads).toEqual([
    { path: "src/a.rs", revision: "git:abc1234" },
    { path: "src/a.rs", revision: "working" },
  ]);
});
