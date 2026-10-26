import { describe, expect, test } from "bun:test";

import type { Extension, ExtensionPage, PageParam, Page } from "../tauri-bridge/generated/bindings.js";
import { fakeAction, fakePage, node } from "./testing/fakeRun.js";
import {
  chooseRenderer,
  chooseRoute,
  openWithOptions,
  pageFileOf,
  pageFileRenderers,
  pageFileUri,
  pageRunParams,
  pageViewChange,
} from "./pageFiles.js";
import { rendererFor, rendererName, renderersFor } from "./renderers.js";

const file = (extension: string, slug: string, over: Partial<Page> = {}): Page =>
  fakePage(node.viz("items", "items"), { id: `${extension}/${slug}`, extension, slug, title: slug, ...over });

const ext = (name: string, namespace: string, pageFiles: Page[], enabled = true) =>
  ({ name, namespace, enabled, pageFiles, pages: [] }) as unknown as Extension;

const tasks = file("oxplow-foundation", "tasks");
const prs = file("gh", "open-prs");
const exts = [ext("oxplow-foundation", "oxplow", [tasks]), ext("gh", "gh", [prs])];

describe("a route's page file", () => {
  test("oxplow's by its slug, any other extension's by `<extension>/<slug>`", () => {
    expect(pageFileOf(exts, { extension: null, page: "tasks", params: {} })?.id).toBe("oxplow-foundation/tasks");
    expect(pageFileOf(exts, { extension: "gh", page: "open-prs", params: {} })?.id).toBe("gh/open-prs");
    // oxplow's pages are core-named: never `oxplow-foundation/tasks`.
    expect(pageFileOf(exts, { extension: "oxplow-foundation", page: "tasks", params: {} })).toBeNull();
    expect(pageFileOf(exts, { extension: null, page: "nope", params: {} })).toBeNull();
  });

  test("a disabled extension's page doesn't resolve; while the extensions load it is undefined", () => {
    expect(pageFileOf([ext("gh", "gh", [prs], false)], { extension: "gh", page: "open-prs", params: {} })).toBeNull();
    expect(pageFileOf(null, { extension: null, page: "tasks", params: {} })).toBeUndefined();
  });

  test("its URI is the route the same rule names", () => {
    expect(pageFileUri(exts, tasks)).toBe("oxplow:oxplow/page?page=tasks");
    expect(pageFileUri(exts, prs, { q: "x" })).toBe("oxplow:oxplow/page?page=gh/open-prs&q=x");
  });
});

describe("renderers over page files", () => {
  const editor = file("oxplow-foundation", "editor", { for: ["oxplow/file"], accepts: { rev: "absent" }, default: true });
  const viewer = file("oxplow-foundation", "file-viewer", { for: ["oxplow/file"], accepts: { rev: "present" } });
  const blame = file("acme", "blame", { for: ["oxplow/file"] });
  const all = [ext("oxplow-foundation", "oxplow", [tasks, editor, viewer]), ext("acme", "acme", [blame])];

  test("only the pages that render something, by the same match rules", () => {
    const renderers = pageFileRenderers(all);
    expect(renderers.map((p) => p.id)).toEqual(["oxplow-foundation/editor", "oxplow-foundation/file-viewer", "acme/blame"]);
    expect(renderersFor("oxplow:oxplow/file?path=a.ts", renderers).map((p) => p.id)).toEqual(["oxplow-foundation/editor", "acme/blame"]);
    expect(rendererFor("oxplow:oxplow/file?path=a.ts&rev=git:HEAD", renderers, {})?.id).toBe("oxplow-foundation/file-viewer");
    expect(rendererFor("oxplow:oxplow/file?path=a.ts", renderers, { "oxplow.renderer": "acme/blame" })?.id).toBe("acme/blame");
  });

  test("a page file's renderer name is its id, as a declared page's is `<extension>/<id>`", () => {
    expect(rendererName(blame)).toBe("acme/blame");
  });
});

const param = (name: string, role: PageParam["role"], over: Partial<PageParam> = {}): PageParam =>
  ({ name, label: null, default: null, options: [], type: "text", role, presets: [], ...over }) as PageParam;

describe("a page's params from its URI", () => {
  const page = file("acme", "list", {
    params: [param("project", "resource"), param("q", "filter"), param("item", "selection"), param("path", null as never)],
  });

  test("the `?` params, and the `#` params the page declares as filters or picks", () => {
    expect(pageRunParams(page, { project: "p1" }, { q: "be", item: "i2", node: "items", "oxplow.renderer": "x/y", project: "p9", path: "nope" })).toEqual({
      project: "p1",
      q: "be",
      item: "i2",
    });
  });

  // A renderer's URI carries its resource's own params (`id`, `provider`);
  // the page is run only with those it declares, since the server refuses
  // any other.
  test("a `?` param the page doesn't declare is left out", () => {
    expect(pageRunParams(page, { project: "p1", id: "1", provider: "oxplow" }, {})).toEqual({ project: "p1" });
  });
});

describe("a change of a page's params", () => {
  const page = fakePage(
    node.switch("modes", "mode", [node.viz("list", "items"), node.viz("board", "items")]),
    {
      params: [
        param("project", "resource"),
        param("q", "filter", { default: "" }),
        param("item", "selection"),
        param("mode", "filter", { default: "list" }),
      ],
    },
  );

  test("a filter replaces the view; defaults are left out", () => {
    expect(pageViewChange(page, { project: "p1", q: "a" }, { q: "" })).toEqual({ resource: { project: "p1" }, view: {}, push: false });
    expect(pageViewChange(page, { project: "p1" }, { q: "be" })).toEqual({ resource: { project: "p1" }, view: { q: "be" }, push: false });
  });

  test("a pick, a switch's section and another resource are somewhere to come back from", () => {
    expect(pageViewChange(page, { project: "p1" }, { item: "i1" }).push).toBe(true);
    expect(pageViewChange(page, { project: "p1" }, { item: null }).push).toBe(false);
    expect(pageViewChange(page, { project: "p1" }, { mode: "board" })).toEqual({ resource: { project: "p1" }, view: { mode: "board" }, push: true });
    expect(pageViewChange(page, { project: "p1" }, { project: "p2" })).toEqual({ resource: { project: "p2" }, view: {}, push: true });
  });
});

describe("what draws a tab", () => {
  const declared = (id: string, over: Partial<ExtensionPage> = {}): ExtensionPage =>
    ({ id, extension: "oxplow-foundation", pageRef: `oxplow:oxplow/page?page=${id}`, title: id, label: null, icon: null, category: null, keywords: null, params: [], count: null, for: [], accepts: {}, default: false, pinned: false, comments: false, refresh: false, body: { kind: "lens", lens: `oxplow-foundation/${id}` }, rail: null, ...over }) as ExtensionPage;
  const formPage = file("oxplow-foundation", "new-task", { view: node.viz("form", null, { viz: "form" }) });
  const commit = file("oxplow-foundation", "commit", { for: ["oxplow/commit"], default: true });
  const editorFile = file("oxplow-foundation", "editor", { for: ["oxplow/file"], default: true, view: node.component("c", "x:c") });
  const exts2 = [
    { ...ext("oxplow-foundation", "oxplow", [tasks, formPage, commit, editorFile]), pages: [declared("tasks"), declared("new-task"), declared("commit", { for: ["oxplow/commit"], default: true }), declared("editor", { for: ["oxplow/file"], default: true }), declared("lens", { for: ["oxplow/lens"], default: true, body: { kind: "builtIn", name: "lens" } }), declared("resource", { body: { kind: "builtIn", name: "resource" } })] } as unknown as Extension,
  ];
  const decls = exts2.flatMap((e) => e.pages);

  // A form, a component, a lazy tree, an expand, an options_from or a form
  // action draws in a page file too: only a lens mounted in its slots
  // keeps a page on its `pages:` entry.
  test("a route: its page file — a form's too — and a `pages:` entry only for what has no file", () => {
    expect(chooseRoute(exts2, { extension: null, page: "tasks", params: {} })).toEqual({ file: tasks, declared: null });
    expect(chooseRoute(exts2, { extension: null, page: "new-task", params: {} })).toEqual({ file: formPage, declared: null });
    expect(chooseRoute(exts2, { extension: null, page: "gone", params: {} })).toEqual({ file: null, declared: null });
  });

  test("a resource: its page file renderer first — a component's too — a `pages:` renderer only for what has none", () => {
    expect(chooseRenderer(exts2, "oxplow:oxplow/commit?sha=abc", {})).toEqual({ file: commit, declared: null });
    expect(chooseRenderer(exts2, "oxplow:oxplow/file?path=a", {})).toEqual({ file: editorFile, declared: null });
    expect(chooseRenderer(exts2, "oxplow:oxplow/lens?lens=a/b", {}).declared?.id).toBe("lens");
    expect(chooseRenderer(exts2, "oxplow:oxplow/nothing?id=1", {})).toEqual({ file: null, declared: null });
  });

  test("Open With lists every renderer once by name, page files first", () => {
    const blame = file("acme", "blame", { title: "Blame", for: ["oxplow/commit"] });
    const all = [...exts2, ext("acme", "acme", [blame])];
    expect(openWithOptions(all, "oxplow:oxplow/commit?sha=abc").map((o) => o.name)).toEqual(["oxplow-foundation/commit", "acme/blame"]);
  });
});

test("a core-named slug two core extensions both have is the one its `pages:` entry names", () => {
  const bundled = { ...ext("oxplow-bundled", "oxplow", [file("oxplow-bundled", "comments", { title: "panel" })]), pages: [] } as unknown as Extension;
  const foundation = { ...ext("oxplow-foundation", "oxplow", [file("oxplow-foundation", "comments", { title: "inbox" })]), pages: [{ id: "comments" }] } as unknown as Extension;
  expect(pageFileOf([bundled, foundation], { extension: null, page: "comments", params: {} })?.title).toBe("inbox");
});

test("a page another extension mounts a lens into stays on its lens until the mount names a page", () => {
  const tasksFile = file("oxplow-foundation", "tasks", { view: node.stack("s", [node.viz("list", "list"), node.slot("sections", "sections")]) });
  const decl = { id: "tasks", extension: "oxplow-foundation", pageRef: "oxplow:oxplow/page?page=tasks", body: { kind: "lens", lens: "oxplow-foundation/tasks-doc" }, for: [], accepts: {}, default: false };
  const foundation = { ...ext("oxplow-foundation", "oxplow", [tasksFile]), pages: [decl], ui: { slots: [] } } as unknown as Extension;
  const mounting = (slot: string) => ({ name: "b", namespace: "b", enabled: true, pages: [], pageFiles: [], ui: { slots: [{ slot, lensId: "b/x" }] } }) as unknown as Extension;
  const route = { extension: null, page: "tasks", params: {} };
  expect(chooseRoute([foundation], route).file?.id).toBe("oxplow-foundation/tasks");
  expect(chooseRoute([foundation, mounting("oxplow-foundation/tasks-doc.sections")], route).declared?.id).toBe("tasks");
  expect(chooseRoute([foundation, mounting("oxplow-foundation/tasks.sections")], route).declared?.id).toBe("tasks");
  expect(chooseRoute([foundation, mounting("oxplow-foundation/other.sections")], route).file?.id).toBe("oxplow-foundation/tasks");
});
