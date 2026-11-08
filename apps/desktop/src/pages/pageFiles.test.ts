import { describe, expect, test } from "bun:test";

import type { Extension, PageParam, Page, Renderer } from "../tauri-bridge/generated/bindings.js";
import { fakeAction, fakePage, node } from "./testing/fakeRun.js";
import {
  chooseRenderer,
  chooseRoute,
  pageFileById,
  pageFileOf,
  pageFileUri,
  pageRunParams,
  pageViewChange,
} from "./pageFiles.js";

const file = (extension: string, slug: string, over: Partial<Page> = {}): Page =>
  fakePage(node.viz("items", "items"), { id: `${extension.startsWith("oxplow-") ? "oxplow" : extension.replace(/-/g, "_")}/${slug}`, extension, slug, title: slug, ...over });

const ext = (name: string, namespace: string, pageFiles: Page[], enabled = true) =>
  ({ name, namespace, enabled, pageFiles }) as unknown as Extension;

const tasks = file("oxplow-foundation", "tasks");
const prs = file("gh", "open-prs");
const exts = [ext("oxplow-foundation", "oxplow", [tasks]), ext("gh", "gh", [prs])];

describe("a route's page file", () => {
  test("oxplow's by its slug, any other's by `<namespace>/<slug>`", () => {
    expect(pageFileOf(exts, { namespace: null, page: "tasks", params: {} })?.id).toBe("oxplow/tasks");
    expect(pageFileOf(exts, { namespace: "gh", page: "open-prs", params: {} })?.id).toBe("gh/open-prs");
    // oxplow's pages are core-named; spelled `oxplow/tasks`, it is the same page.
    expect(pageFileOf(exts, { namespace: "oxplow", page: "tasks", params: {} })?.id).toBe("oxplow/tasks");
    expect(pageFileOf(exts, { namespace: null, page: "nope", params: {} })).toBeNull();
  });

  test("a disabled extension's page doesn't resolve; while the extensions load it is undefined", () => {
    expect(pageFileOf([ext("gh", "gh", [prs], false)], { namespace: "gh", page: "open-prs", params: {} })).toBeNull();
    expect(pageFileOf(null, { namespace: null, page: "tasks", params: {} })).toBeUndefined();
  });

  test("its URI is the route the same rule names", () => {
    expect(pageFileUri(exts, tasks)).toBe("oxplow:oxplow/page?page=tasks");
    expect(pageFileUri(exts, prs, { q: "x" })).toBe("oxplow:oxplow/page?page=gh/open-prs&q=x");
  });
});

describe("a renderer's page file", () => {
  const editor = file("oxplow-foundation", "editor", { for: ["oxplow/file"] });
  const blame = file("acme", "blame", { for: ["oxplow/file"] });
  const all = [ext("oxplow-foundation", "oxplow", [tasks, editor]), ext("acme", "acme", [blame], false)];

  test("is found by its id among the enabled extensions' pages", () => {
    expect(pageFileById(all, "oxplow/editor")).toBe(editor);
    // Disabled, or not there: nothing.
    expect(pageFileById(all, "acme/blame")).toBeNull();
    expect(pageFileById(all, "acme/gone")).toBeNull();
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

describe("a view param", () => {
  const page = file("acme", "shown", { params: [param("uri", "resource"), param("mode", "view")] });

  test("is read from the `#`, never the `?`", () => {
    expect(pageRunParams(page, { uri: "u" }, { mode: "raw" })).toEqual({ uri: "u", mode: "raw" });
    expect(pageRunParams(page, { uri: "u", mode: "nope" }, {})).toEqual({ uri: "u", mode: "nope" });
  });

  test("a change replaces the view, like a filter", () => {
    expect(pageViewChange(page, { uri: "u" }, { mode: "raw" })).toEqual({ resource: { uri: "u" }, view: { mode: "raw" }, push: false });
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
    expect(chooseRoute(exts2, { namespace: null, page: "tasks", params: {} })).toBe(tasks);
    expect(chooseRoute(exts2, { namespace: null, page: "new-task", params: {} })).toBe(formPage);
    expect(chooseRoute(exts2, { namespace: null, page: "gone", params: {} })).toBeNull();
  });

  const offer = (...pages: Page[]): Renderer[] => pages.map((p) => ({ page: p.id, title: p.title, default: p.default }));

  test("a resource: the page file of the renderer the daemon offered — a component's too — else nothing (the fallback draws it)", () => {
    expect(chooseRenderer(exts2, offer(commit), {})).toBe(commit);
    expect(chooseRenderer(exts2, offer(editorFile), {})).toBe(editorFile);
    expect(chooseRenderer(exts2, [], {})).toBeNull();
    // The view's pick, among what was offered.
    expect(chooseRenderer(exts2, offer(editorFile, commit), { "oxplow.renderer": "oxplow/commit" })).toBe(commit);
    // Offered, but no longer a page file (the extensions just reloaded): the fallback.
    expect(chooseRenderer(exts2, [{ page: "acme/gone", title: "Gone", default: true }], {})).toBeNull();
  });

  // Mounts in a page's slots never move it off its file.
  test("a page others mount into is still drawn from its file", () => {
    const mounting = { name: "b", namespace: "b", enabled: true, pageFiles: [], ui: { slots: [{ slot: "oxplow/tasks.sections" }] } } as unknown as Extension;
    expect(chooseRoute([...exts2, mounting], { namespace: null, page: "tasks", params: {} })).toBe(tasks);
  });
});
