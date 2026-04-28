import { expect, test } from "bun:test";
import { parseMarkdownLink, preprocessWikilinks } from "./MarkdownView.js";

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

test("parseMarkdownLink: file: scheme with plain path", () => {
  expect(parseMarkdownLink("file:src/foo.ts")).toEqual({ kind: "file", path: "src/foo.ts" });
});

test("parseMarkdownLink: file: scheme with line suffix", () => {
  expect(parseMarkdownLink("file:src/foo.ts:42")).toEqual({ kind: "file", path: "src/foo.ts", line: 42 });
});

test("parseMarkdownLink: file: scheme with empty target", () => {
  expect(parseMarkdownLink("file:")).toEqual({ kind: "empty" });
});

test("preprocessWikilinks: rewrites file path target to file: link", () => {
  expect(preprocessWikilinks("see [[src/foo.ts]] for context"))
    .toBe("see [src/foo.ts](file:src/foo.ts) for context");
});

test("preprocessWikilinks: file path with line suffix", () => {
  expect(preprocessWikilinks("see [[src/foo.ts:88]]"))
    .toBe("see [src/foo.ts:88](file:src/foo.ts:88)");
});

test("preprocessWikilinks: |display syntax", () => {
  expect(preprocessWikilinks("the [[src/foo.ts|foo helper]] does X"))
    .toBe("the [foo helper](file:src/foo.ts) does X");
});

test("preprocessWikilinks: bare slug routes to wiki note", () => {
  expect(preprocessWikilinks("see [[architecture]] note"))
    .toBe("see [architecture](architecture) note");
});

test("preprocessWikilinks: leaves wikilinks inside fenced code untouched", () => {
  const input = "Use this:\n```\n[[src/foo.ts]]\n```\nbut [[src/bar.ts]] is rewritten.";
  const out = preprocessWikilinks(input);
  expect(out).toContain("```\n[[src/foo.ts]]\n```");
  expect(out).toContain("[src/bar.ts](file:src/bar.ts)");
});

test("preprocessWikilinks: leaves wikilinks inside inline backticks untouched", () => {
  expect(preprocessWikilinks("use the literal `[[path]]` syntax"))
    .toBe("use the literal `[[path]]` syntax");
});

test("preprocessWikilinks: handles multiple wikilinks on one line", () => {
  expect(preprocessWikilinks("[[src/a.ts]] and [[src/b.ts]] interact"))
    .toBe("[src/a.ts](file:src/a.ts) and [src/b.ts](file:src/b.ts) interact");
});

test("preprocessWikilinks: a target with no extension and no slash is a slug", () => {
  expect(preprocessWikilinks("[[architecture]]"))
    .toBe("[architecture](architecture)");
});

test("preprocessWikilinks: nested-looking brackets do not match", () => {
  // Unmatched / pathological — just ensure we don't crash and don't rewrite
  // when target contains brackets.
  expect(preprocessWikilinks("[[ ]]"))
    .toBe("[[ ]]");
});

test("preprocessWikilinks: bare 7-char hex resolves to a git-commit link", () => {
  // The display text shrinks to the canonical short SHA so a 40-char raw
  // target doesn't blow up inline prose.
  expect(preprocessWikilinks("introduced in [[abc1234]]"))
    .toBe("introduced in [abc1234](gitcommit:abc1234)");
});

test("preprocessWikilinks: full 40-char SHA renders short display, full target", () => {
  const sha = "0123456789abcdef0123456789abcdef01234567";
  expect(preprocessWikilinks(`see [[${sha}]]`))
    .toBe(`see [0123456](gitcommit:${sha})`);
});

test("preprocessWikilinks: explicit git: prefix", () => {
  expect(preprocessWikilinks("[[git:deadbeef]]"))
    .toBe("[deadbee](gitcommit:deadbeef)");
});

test("preprocessWikilinks: |display label overrides short-sha shrinking", () => {
  expect(preprocessWikilinks("[[abc1234|the migration commit]]"))
    .toBe("[the migration commit](gitcommit:abc1234)");
});

test("preprocessWikilinks: SHA detection is case-insensitive and normalizes to lowercase", () => {
  // Display text + href both normalize to lowercase so two notes spelling
  // the same sha differently render identically and hit the same tab.
  expect(preprocessWikilinks("[[ABC1234]]"))
    .toBe("[abc1234](gitcommit:abc1234)");
});

test("preprocessWikilinks: 6-char hex is too short to be a SHA — treated as slug", () => {
  expect(preprocessWikilinks("[[abc123]]"))
    .toBe("[abc123](abc123)");
});

test("preprocessWikilinks: hex with non-hex chars is a slug", () => {
  expect(preprocessWikilinks("[[abc-1234]]"))
    .toBe("[abc-1234](abc-1234)");
});

test("parseMarkdownLink: gitcommit: scheme", () => {
  expect(parseMarkdownLink("gitcommit:abc1234")).toEqual({ kind: "git-commit", sha: "abc1234" });
});

test("parseMarkdownLink: gitcommit: with empty target", () => {
  expect(parseMarkdownLink("gitcommit:")).toEqual({ kind: "empty" });
});
