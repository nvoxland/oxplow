import { expect, test } from "bun:test";
import { parseMarkdownLink } from "./MarkdownView.js";

// parseMarkdownLink is shared by NoteTab and WorkItemDetail. NoteTab needs
// to distinguish wiki-internal links (`./foo`, `bar.md`) from external
// (`https://…`) and anchor (`#section`) links so it can route plain
// clicks back through the wiki navigation history. WorkItemDetail just
// needs to know "is this an external link" so it can route to the OS
// browser; internal slug semantics don't apply there but the same parser
// works.

test("parseMarkdownLink: empty href", () => {
  expect(parseMarkdownLink("")).toEqual({ kind: "empty" });
});

test("parseMarkdownLink: anchor", () => {
  expect(parseMarkdownLink("#section")).toEqual({ kind: "anchor" });
});

test("parseMarkdownLink: external http(s) and mailto", () => {
  expect(parseMarkdownLink("https://example.com")).toEqual({ kind: "external" });
  expect(parseMarkdownLink("http://example.com/path")).toEqual({ kind: "external" });
  expect(parseMarkdownLink("mailto:nathan@voxland.net")).toEqual({ kind: "external" });
});

test("parseMarkdownLink: internal — strips leading ./, .md suffix, query, fragment", () => {
  expect(parseMarkdownLink("foo")).toEqual({ kind: "internal", slug: "foo" });
  expect(parseMarkdownLink("./foo")).toEqual({ kind: "internal", slug: "foo" });
  expect(parseMarkdownLink("/foo")).toEqual({ kind: "internal", slug: "foo" });
  expect(parseMarkdownLink("foo.md")).toEqual({ kind: "internal", slug: "foo" });
  expect(parseMarkdownLink("foo.md#sec")).toEqual({ kind: "internal", slug: "foo" });
  expect(parseMarkdownLink("foo.md?x=1")).toEqual({ kind: "internal", slug: "foo" });
});

test("parseMarkdownLink: internal that resolves to empty slug", () => {
  expect(parseMarkdownLink("./")).toEqual({ kind: "empty" });
});
