import { describe, expect, test } from "bun:test";

import type { Extension, PageParam, Page } from "../tauri-bridge/generated/bindings.js";
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
  ({ name, namespace, enabled, pageFiles }) as unknown as Extension;

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
  const editor = file("oxplow-foundation", "editor", { for: ["oxplow/file"], accepts: { rev: "absent", filesystem: "absent" }, default: true });
  const viewer = file("oxplow-foundation", "file-viewer", { for: ["oxplow/file"] });
  const blame = file("acme", "blame", { for: ["oxplow/file"] });
  const all = [ext("oxplow-foundation", "oxplow", [tasks, editor, viewer]), ext("acme", "acme", [blame])];

  test("only the pages that render something, by the same match rules", () => {
    const renderers = pageFileRenderers(all);
    expect(renderers.map((p) => p.id)).toEqual(["oxplow-foundation/editor", "oxplow-foundation/file-viewer", "acme/blame"]);
    expect(renderersFor("oxplow:oxplow/file?path=a.ts", renderers).map((p) => p.id)).toEqual(["oxplow-foundation/editor", "oxplow-foundation/file-viewer", "acme/blame"]);
    expect(renderersFor("oxplow:oxplow/file?path=a.ts&filesystem=oxplow:temp", renderers).map((p) => p.id)).toEqual(["oxplow-foundation/file-viewer", "acme/blame"]);
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
  const formPage = file("oxplow-foundation", "new-task", { view: node.viz("form", null, { viz: "form" }) });
  const commit = file("oxplow-foundation", "commit", { for: ["oxplow/commit"], default: true });
  const editorFile = file("oxplow-foundation", "editor", { for: ["oxplow/file"], default: true, view: node.component("c", "x:c") });
  const exts2 = [ext("oxplow-foundation", "oxplow", [tasks, formPage, commit, editorFile])];

  test("a route: its page file — a form's too — else nothing", () => {
    expect(chooseRoute(exts2, { extension: null, page: "tasks", params: {} })).toBe(tasks);
    expect(chooseRoute(exts2, { extension: null, page: "new-task", params: {} })).toBe(formPage);
    expect(chooseRoute(exts2, { extension: null, page: "gone", params: {} })).toBeNull();
  });

  test("a resource: its page file renderer — a component's too — else nothing (the fallback draws it)", () => {
    expect(chooseRenderer(exts2, "oxplow:oxplow/commit?sha=abc", {})).toBe(commit);
    expect(chooseRenderer(exts2, "oxplow:oxplow/file?path=a", {})).toBe(editorFile);
    expect(chooseRenderer(exts2, "oxplow:oxplow/page?page=a/b", {})).toBeNull();
    expect(chooseRenderer(exts2, "oxplow:oxplow/nothing?id=1", {})).toBeNull();
  });

  test("Open With lists every renderer once by name", () => {
    const blame = file("acme", "blame", { title: "Blame", for: ["oxplow/commit"] });
    const all = [...exts2, ext("acme", "acme", [blame])];
    expect(openWithOptions(all, "oxplow:oxplow/commit?sha=abc").map((o) => o.name)).toEqual(["oxplow-foundation/commit", "acme/blame"]);
  });

  // Mounts in a page's slots never move it off its file.
  test("a page others mount into is still drawn from its file", () => {
    const mounting = { name: "b", namespace: "b", enabled: true, pageFiles: [], ui: { slots: [{ slot: "oxplow-foundation/tasks.sections" }] } } as unknown as Extension;
    expect(chooseRoute([...exts2, mounting], { extension: null, page: "tasks", params: {} })).toBe(tasks);
  });
});
