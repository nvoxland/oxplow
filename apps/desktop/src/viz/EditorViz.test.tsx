import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Stream } from "../api.js";
import { createBufferStore, type ModelHandle } from "../editor/buffers.js";
import { PageNavigationContext, type PageNavigation, type TabEditing } from "../tabs/PageNavigationContext.js";

// The editor viz edits the working tree's file its row names: its buffer
// is the window's, it tells its tab when it is dirty and how to save, and
// the disk changing under a draft asks what to keep.

class Model implements ModelHandle {
  cbs: (() => void)[] = [];
  constructor(public text: string) {}
  getValue() {
    return this.text;
  }
  setValue(t: string) {
    this.text = t;
    this.cbs.forEach((c) => c());
  }
  onDidChangeContent(cb: () => void) {
    this.cbs.push(cb);
    return { dispose: () => {} };
  }
  dispose() {}
}

const disk: Record<string, string> = { "src/a.rs": "fn a() {}" };
const saves: { path: string; content: string }[] = [];
const models = new Map<string, Model>();
const store = createBufferStore({
  read: async (_s, path) => disk[path] ?? null,
  save: async (_s, path, content) => {
    saves.push({ path, content });
    disk[path] = content;
  },
  model: async (_s, path, text) => {
    const m = new Model(text);
    models.set(path, m);
    return m;
  },
});
const realOpenFiles = { ...(await import("../editor/openFiles.js")) };
const realEditorPane = { ...(await import("../components/EditorPane.js")) };
afterAll(() => {
  mock.module("../editor/openFiles.js", () => realOpenFiles);
  mock.module("../components/EditorPane.js", () => realEditorPane);
});
mock.module("../editor/openFiles.js", () => ({ buffers: store, loadMonaco: async () => null }));
const panes: { filePath: string; line: number | null }[] = [];
mock.module("../components/EditorPane.js", () => ({
  EditorPane: (props: { filePath: string; line: number | null }) => {
    panes.push({ filePath: props.filePath, line: props.line });
    return <div data-testid="editor-pane">{props.filePath}</div>;
  },
}));

const { EditorViz } = await import("./EditorViz.js");
const { VizStreamContext } = await import("./VizStream.js");

const stream = { id: "str1", worktree_path: "/w" } as Stream;

afterEach(() => {
  cleanup();
  store.retainOnly([]);
  saves.length = 0;
  panes.length = 0;
});

function view(row: { path: string; filesystem?: string | null; line: number | null }, registered: (TabEditing | null)[] = []) {
  const setEditing = (e: TabEditing | null) => registered.push(e);
  const nav = { navigate: () => {}, goBack: () => {}, goForward: () => {}, canGoBack: false, canGoForward: false, setEditing } as PageNavigation;
  return render(
    <PageNavigationContext.Provider value={nav}>
      <VizStreamContext.Provider value={stream}>
        <EditorViz row={row} />
      </VizStreamContext.Provider>
    </PageNavigationContext.Provider>,
  );
}

test("it opens the row's file and shows it, landing on its line", async () => {
  const v = view({ path: "src/a.rs", line: 3 });
  await waitFor(() => expect(v.getByTestId("editor-pane").textContent).toBe("src/a.rs"));
  expect(panes.at(-1)).toEqual({ filePath: "src/a.rs", line: 3 });
});

test("typing makes the tab dirty, and the tab's Save writes the draft", async () => {
  const registered: (TabEditing | null)[] = [];
  const v = view({ path: "src/a.rs", line: null }, registered);
  await waitFor(() => expect(v.getByTestId("editor-pane")).toBeTruthy());
  expect(registered.at(-1)?.dirty).toBe(false);
  act(() => models.get("src/a.rs")!.setValue("fn b() {}"));
  await waitFor(() => expect(registered.at(-1)?.dirty).toBe(true));
  await act(async () => {
    registered.at(-1)!.save();
  });
  await waitFor(() => expect(saves).toEqual([{ path: "src/a.rs", content: "fn b() {}" }]));
  await waitFor(() => expect(registered.at(-1)?.dirty).toBe(false));
});

test("a file of another filesystem is refused, not opened: the editor edits only the working tree", async () => {
  const v = view({ path: "acme/page.yaml", filesystem: "oxplow:extensions", line: null });
  expect(v.getByTestId("viz-editor-filesystem").textContent).toContain("oxplow:extensions");
  expect(panes).toEqual([]);
  expect(models.has("acme/page.yaml")).toBe(false);
  const local = view({ path: "src/a.rs", filesystem: "oxplow:local", line: null });
  await waitFor(() => expect(local.getByTestId("editor-pane")).toBeTruthy());
});

test("a path that isn't a file says so", async () => {
  const v = view({ path: "nope.rs", line: null });
  await waitFor(() => expect(v.getByTestId("viz-editor-missing").textContent).toContain("nope.rs"));
});

test("the disk changing under a draft asks; Reload takes the disk's text", async () => {
  disk["src/a.rs"] = "start";
  const v = view({ path: "src/a.rs", line: null });
  await waitFor(() => expect(v.getByTestId("editor-pane")).toBeTruthy());
  act(() => models.get("src/a.rs")!.setValue("mine"));
  disk["src/a.rs"] = "theirs";
  await act(async () => {
    await store.diskChanged("str1", "src/a.rs");
  });
  fireEvent.click(v.getByTestId("viz-editor-reload"));
  await waitFor(() => expect(v.queryByTestId("viz-editor-conflict")).toBeNull());
  expect(models.get("src/a.rs")!.getValue()).toBe("theirs");
});
