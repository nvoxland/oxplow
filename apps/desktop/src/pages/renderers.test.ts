import { describe, expect, test } from "bun:test";

import type { ExtensionPage } from "../tauri-bridge/generated/bindings.js";
import { RENDERER_VIEW_KEY, pageView, renderedResourceOf, rendererFor, renderersFor, shellView } from "./renderers.js";

const page = (id: string, more: Partial<ExtensionPage> = {}): ExtensionPage => ({
  id,
  extension: "oxplow-foundation",
  pageRef: `oxplow:oxplow/page?page=${id}`,
  title: id,
  icon: null,
  category: null,
  keywords: null,
  params: [],
  count: null,
  for: [],
  accepts: {},
  default: false,
  pinned: false,
  body: { kind: "builtIn", name: id },
  ...more,
});
const editor = page("editor", { for: ["oxplow/file"], accepts: { rev: "absent" }, default: true });
const viewer = page("file-viewer", { for: ["oxplow/file"], accepts: { rev: "present" } });
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
    expect(renderersFor("oxplow:oxplow/file?path=a.ts", pages).map((p) => p.id)).toEqual(["editor", "blame"]);
    expect(renderersFor("oxplow:oxplow/file?path=a.ts&rev=git:HEAD", pages).map((p) => p.id)).toEqual(["file-viewer", "blame"]);
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
