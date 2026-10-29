import { describe, expect, test } from "bun:test";

import type { Page } from "../tauri-bridge/generated/bindings.js";
import { fakePage, node } from "./testing/fakeRun.js";
import { RENDERER_VIEW_KEY, pageView, renderedResourceOf, rendererFor, renderersFor, shellView, tabLabel, withRenderedParams } from "./renderers.js";

const page = (id: string, more: Partial<Page> = {}): Page =>
  fakePage(node.markdown("v", ""), { id, extension: "oxplow-foundation", slug: id, title: id, ...more });
const editor = page("editor", { for: ["oxplow/file"], accepts: { rev: "absent", filesystem: "absent" }, default: true });
const viewer = page("file-viewer", { for: ["oxplow/file"] });
const blame = page("blame", { extension: "acme", for: ["oxplow/file"] });
const prs = page("pr", { extension: "acme", for: ["acme/pr"], accepts: { id: "^[0-9]+$" } });
const web = page("external-url", { for: ["https", "http"], default: true });
const pages = [editor, viewer, blame, prs, web, page("tasks")];

describe("renderedResourceOf", () => {
  test("an oxplow URI's resource, an http(s) URL's scheme, else nothing", () => {
    expect(renderedResourceOf("oxplow:oxplow/file?path=a.ts")).toBe("oxplow/file");
    expect(renderedResourceOf("https://example.com/a")).toBe("https");
    expect(renderedResourceOf("ftp://example.com/a")).toBeNull();
    expect(renderedResourceOf("not a uri")).toBeNull();
  });
});

describe("renderers", () => {
  test("the pages for a URI's resource whose `accepts` hold for its params", () => {
    expect(renderersFor("oxplow:oxplow/file?path=a.ts", pages).map((p) => p.id)).toEqual(["editor", "file-viewer", "blame"]);
    expect(renderersFor("oxplow:oxplow/file?path=a.ts&rev=git:HEAD", pages).map((p) => p.id)).toEqual(["file-viewer", "blame"]);
    // The editor edits only the local working tree: another filesystem's file is the viewer's.
    expect(renderersFor("oxplow:oxplow/file?path=a.ts&filesystem=oxplow:temp", pages).map((p) => p.id)).toEqual(["file-viewer", "blame"]);
    expect(renderersFor("oxplow:acme/pr?id=12", pages).map((p) => p.id)).toEqual(["pr"]);
    expect(renderersFor("oxplow:acme/pr?id=x", pages)).toEqual([]);
    expect(renderersFor("https://example.com", pages).map((p) => p.id)).toEqual(["external-url"]);
  });

  test("the view's renderer, else the default, else the one there is, else none", () => {
    expect(rendererFor("oxplow:oxplow/file?path=a.ts", pages, {})?.id).toBe("editor");
    expect(rendererFor("oxplow:oxplow/file?path=a.ts", pages, { [RENDERER_VIEW_KEY]: "acme/blame" })?.id).toBe("blame");
    // A renderer the view names that doesn't take this URI is ignored.
    expect(rendererFor("oxplow:oxplow/file?path=a.ts", pages, { [RENDERER_VIEW_KEY]: "acme/pr" })?.id).toBe("editor");
    // No default among them: the first, as the extensions list them.
    expect(rendererFor("oxplow:oxplow/file?path=a.ts&rev=git:HEAD", pages, {})?.id).toBe("file-viewer");
    expect(rendererFor("oxplow:acme/pr?id=12", pages, {})?.id).toBe("pr");
    expect(rendererFor("oxplow:oxplow/commit?sha=abc", pages, {})).toBeNull();
  });
});

// The renderer pick is the shell's: a page never reads it as its own
// param, and writing its own view keeps it.
test("a view splits into the shell's keys and the page's", () => {
  const view = { [RENDERER_VIEW_KEY]: "acme/blame", q: "x", lines: "3-3" };
  expect(shellView(view)).toEqual({ [RENDERER_VIEW_KEY]: "acme/blame" });
  expect(pageView(view)).toEqual({ q: "x", lines: "3-3" });
});

// A renderer's page is its URI: another value of one of its params is the
// same resource with that param changed (its page's own `uri` isn't one),
// one set to nothing is dropped, and the view rides along.
test("a renderer's params change the URI it renders", () => {
  const uri = "oxplow:oxplow/dir?path=src";
  expect(withRenderedParams(uri, { uri, path: "src/lib" }, {})).toBe("oxplow:oxplow/dir?path=src/lib");
  expect(withRenderedParams("oxplow:acme/pr?id=1&repo=a", { id: 2 }, { tab: "files" })).toBe("oxplow:acme/pr?id=2&repo=a#tab=files");
  expect(withRenderedParams("oxplow:acme/pr?id=1&repo=a", { repo: null }, {})).toBe("oxplow:acme/pr?id=1");
  expect(withRenderedParams("https://example.com", { id: 2 }, {})).toBe("https://example.com");
});

// A page's tab says its `label`, filled from what it shows; a label that
// fills to nothing, or none, says its title.
test("a page's tab label is its label filled from its params, else its title", () => {
  const dir = page("directory", { title: "Directory", label: "{{param.path}}" });
  expect(tabLabel(dir, { path: "src/lib" })).toBe("src/lib");
  expect(tabLabel(dir, {})).toBe("Directory");
  expect(tabLabel(page("pr", { title: "PR", label: "PR #{{ param.id }}" }), { id: "12" })).toBe("PR #12");
  expect(tabLabel(page("tasks", { title: "Tasks" }), { id: "12" })).toBe("Tasks");
});

// A label may also name a column of its page's row, and filter what it
// fills: `short` (7 characters), `basename` (the last path segment).
test("a label fills row columns and applies its filters; with no row yet it says the title", () => {
  const dash = page("dash", { title: "Dashboard", label: "{{row.title}}" });
  expect(tabLabel(dash, {}, { title: "Build health" })).toBe("Build health");
  // The run hasn't answered: the static label, never a half-filled one.
  expect(tabLabel(dash, {})).toBe("Dashboard");
  expect(tabLabel(dash, {}, null)).toBe("Dashboard");
  const commit = page("commit", { title: "Commit", label: "{{param.sha|short}}" });
  expect(tabLabel(commit, { sha: "0123456789abcdef" })).toBe("0123456");
  const file = page("file", { title: "File", label: "{{ param.path | basename }} ({{param.rev|short}})" });
  expect(tabLabel(file, { path: "src/lib/a.rs", rev: "git:abcdef0123" })).toBe("a.rs (git:abc)");
  const both = page("both", { title: "Both", label: "{{row.title|basename}} · {{param.id}}" });
  expect(tabLabel(both, { id: "7" }, { title: "x/y" })).toBe("y · 7");
});

test("the url filter is a web page's host and path, cut to 40", () => {
  const web = page("external-url", { title: "Web Page", label: "{{param.uri|url}}" });
  expect(tabLabel(web, { uri: "https://example.com/" })).toBe("example.com");
  expect(tabLabel(web, { uri: "https://docs.example.com/guide/start?x=1#top" })).toBe("docs.example.com/guide/start");
  const long = tabLabel(web, { uri: `https://example.com/${"a".repeat(80)}` });
  expect(long.length).toBe(40);
  expect(long.endsWith("…")).toBe(true);
  // Not a URL: kept as it is.
  expect(tabLabel(web, { uri: "nonsense" })).toBe("nonsense");
});

// A page file's `{{row.*}}` reads its first loader's first row.
test("labelRow is a page run's first loader's first row; a page file's label fills from it", async () => {
  const { labelRow } = await import("./renderers.js");
  const { decl, fakePage, fakeRun, loader, node } = await import("./testing/fakeRun.js");
  const pf = fakePage(node.viz("v", "commit"), { title: "Commit", label: "{{row.subject}} ({{param.sha|short}})", loaders: [decl("commit"), decl("files")] });
  const run = fakeRun(pf, {
    params: { sha: "abcdef123456" },
    loaders: { files: loader(["subject"], [["not this"]]), commit: loader(["subject"], [["Fix it"], ["second"]]) },
  });
  expect(labelRow(run)).toEqual({ subject: "Fix it" });
  expect(tabLabel(pf, run.params, labelRow(run))).toBe("Fix it (abcdef1)");
  expect(labelRow({ ...run, loaders: { commit: loader(["subject"], []) } })).toBeNull();
  expect(labelRow({ ...run, page: { ...pf, loaders: [] } })).toBeNull();
});
