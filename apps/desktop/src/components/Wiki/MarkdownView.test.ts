import { expect, test } from "bun:test";
import { parseMarkdownLink, postprocessWikilinks, preprocessWikilinks } from "./MarkdownView.js";

// parseMarkdownLink is shared by WikiPageTab and TaskDetail. WikiPageTab needs
// to distinguish wiki-internal links (`./foo`, `bar.md`) from external
// (`https://…`) and anchor (`#section`) links so it can route plain
// clicks back through the wiki navigation history. TaskDetail just
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
  expect(parseMarkdownLink("file:src/foo.ts")).toEqual({ kind: "file", path: "src/foo.ts", version: null });
});

test("parseMarkdownLink: file: scheme with line suffix", () => {
  expect(parseMarkdownLink("file:src/foo.ts:42")).toEqual({ kind: "file", path: "src/foo.ts", line: 42, version: null });
});

test("parseMarkdownLink: file: scheme with @disk version", () => {
  expect(parseMarkdownLink("file:src/foo.ts@disk")).toEqual({
    kind: "file",
    path: "src/foo.ts",
    version: { kind: "disk" },
  });
});

test("parseMarkdownLink: file: scheme with @<sha> version", () => {
  expect(parseMarkdownLink("file:src/foo.ts@abc1234")).toEqual({
    kind: "file",
    path: "src/foo.ts",
    version: { kind: "ref", ref: "abc1234" },
  });
});

test("parseMarkdownLink: file: scheme with @HEAD version + line", () => {
  expect(parseMarkdownLink("file:src/foo.ts@HEAD:42")).toEqual({
    kind: "file",
    path: "src/foo.ts",
    line: 42,
    version: { kind: "ref", ref: "HEAD" },
  });
});

test("parseMarkdownLink: file: scheme with @local alias", () => {
  expect(parseMarkdownLink("file:src/foo.ts@local")).toEqual({
    kind: "file",
    path: "src/foo.ts",
    version: { kind: "disk" },
  });
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

test("preprocessWikilinks: file path with @version is preserved verbatim", () => {
  // The version segment passes through to the file: URL; the click
  // handler decodes it back into a FileVersion via parseMarkdownLink.
  expect(preprocessWikilinks("see [[src/foo.ts@HEAD]]"))
    .toBe("see [src/foo.ts@HEAD](file:src/foo.ts@HEAD)");
  expect(preprocessWikilinks("see [[src/foo.ts@disk:42]]"))
    .toBe("see [src/foo.ts@disk:42](file:src/foo.ts@disk:42)");
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

test("parseMarkdownLink: dir: scheme strips trailing slash", () => {
  expect(parseMarkdownLink("dir:src/components")).toEqual({ kind: "directory", path: "src/components" });
  expect(parseMarkdownLink("dir:src/components/")).toEqual({ kind: "directory", path: "src/components" });
});

test("parseMarkdownLink: dir: with empty target", () => {
  expect(parseMarkdownLink("dir:")).toEqual({ kind: "empty" });
});

test("preprocessWikilinks: dir: prefix rewrites to dir: href", () => {
  expect(preprocessWikilinks("see [[dir:src/components]] for the buttons"))
    .toBe("see [src/components](dir:src/components) for the buttons");
});

test("preprocessWikilinks: dir: prefix tolerates trailing slash on the path", () => {
  expect(preprocessWikilinks("[[dir:src/components/]]"))
    .toBe("[src/components](dir:src/components)");
});

test("preprocessWikilinks: dir: prefix with custom display label", () => {
  expect(preprocessWikilinks("[[dir:src/components|the components folder]]"))
    .toBe("[the components folder](dir:src/components)");
});

// postprocessWikilinks is the inverse used on RichTextField → disk.
// It converts standard markdown links with our internal schemes back
// into `[[ ]]` wikilink form so wiki pages keep their authored shape.

test("postprocessWikilinks: file link with matching label collapses to bare wikilink", () => {
  expect(postprocessWikilinks("see [src/foo.ts](file:src/foo.ts) here"))
    .toBe("see [[src/foo.ts]] here");
});

test("postprocessWikilinks: file link with distinct label preserves the label", () => {
  expect(postprocessWikilinks("see [the helper](file:src/foo.ts) here"))
    .toBe("see [[src/foo.ts|the helper]] here");
});

test("postprocessWikilinks: dir link", () => {
  expect(postprocessWikilinks("[src/components](dir:src/components)"))
    .toBe("[[dir:src/components]]");
  expect(postprocessWikilinks("[the components folder](dir:src/components)"))
    .toBe("[[dir:src/components|the components folder]]");
});

test("postprocessWikilinks: gitcommit link with short-sha label drops the label", () => {
  expect(postprocessWikilinks("see [abc1234](gitcommit:abc1234deadbeef) here"))
    .toBe("see [[git:abc1234deadbeef]] here");
});

test("postprocessWikilinks: gitcommit link with custom label preserves it", () => {
  expect(postprocessWikilinks("see [the fix](gitcommit:abc1234deadbeef) here"))
    .toBe("see [[git:abc1234deadbeef|the fix]] here");
});

test("postprocessWikilinks: leaves plain http/https/internal-slug links alone", () => {
  expect(postprocessWikilinks("see [docs](https://example.com) plus [arch](architecture)"))
    .toBe("see [docs](https://example.com) plus [arch](architecture)");
});

test("postprocessWikilinks: skips image links and inline code", () => {
  const src = "![alt](file:foo.png) and `[label](file:foo.ts)` literal";
  expect(postprocessWikilinks(src)).toBe(src);
});

test("postprocessWikilinks: skips fenced code blocks", () => {
  const src = "before\n```\n[label](file:foo.ts)\n```\nafter [label](file:foo.ts)";
  expect(postprocessWikilinks(src))
    .toBe("before\n```\n[label](file:foo.ts)\n```\nafter [[foo.ts|label]]");
});

// markdown-it's default validateLink rejects file:/data: URLs, so when
// the editor receives `[label](file:path)` it ends up storing them as
// escaped literal text and serializing them back as `\[[path|path\]]`
// (escaped outer brackets, redundant duplicated label). postprocess
// must defensively normalize that mangled shape back to a bare
// wikilink, or the on-disk file rots a little more on every save.
test("postprocessWikilinks: cleans up the editor's escaped-bracket round-trip", () => {
  // Real-world shape observed after a wiki save round-trip:
  expect(postprocessWikilinks("see \\[[src/foo.ts|src/foo.ts\\]] here"))
    .toBe("see [[src/foo.ts]] here");
});

test("postprocessWikilinks: escaped-bracket round-trip preserves a real label", () => {
  expect(postprocessWikilinks("\\[[src/foo.ts|the helper\\]]"))
    .toBe("[[src/foo.ts|the helper]]");
});

test("postprocessWikilinks: escaped-bracket cleanup handles multiple occurrences", () => {
  expect(postprocessWikilinks("\\[[a.ts|a.ts\\]] and \\[[b.ts|b.ts\\]]"))
    .toBe("[[a.ts]] and [[b.ts]]");
});

test("postprocessWikilinks ∘ preprocessWikilinks is identity for supported forms", () => {
  const samples = [
    "see [[src/foo.ts]] in the codebase",
    "see [[src/foo.ts|the helper]] in the codebase",
    "see [[dir:src/components]] for buttons",
    "see [[dir:src/components|the folder]] for buttons",
    "see [[tsk42]] for the work",
    "see [[tsk42|fix the parser]] for the work",
  ];
  for (const sample of samples) {
    expect(postprocessWikilinks(preprocessWikilinks(sample))).toBe(sample);
  }
});

// Task wikilinks: `[[tsk<id>]]` → `task:` scheme; the renderer swaps the
// `tsk<id>` token for the task title at display time. The backend ref
// extractor (refs.rs) already recognizes the same form for backlinks.

test("preprocessWikilinks: task ref [[tsk42]] rewrites to task: href", () => {
  expect(preprocessWikilinks("see [[tsk42]] for context"))
    .toBe("see [tsk42](task:tsk42) for context");
});

test("preprocessWikilinks: task ref with custom display label", () => {
  expect(preprocessWikilinks("[[tsk42|fix the parser]]"))
    .toBe("[fix the parser](task:tsk42)");
});

test("preprocessWikilinks: non-numeric tsk token is treated as a wiki slug", () => {
  // Only `tsk<digits>` is a task ref; `tsk-notes` is an ordinary slug.
  expect(preprocessWikilinks("[[tsk-notes]]")).toBe("[tsk-notes](tsk-notes)");
});

test("parseMarkdownLink: task: scheme", () => {
  expect(parseMarkdownLink("task:tsk42")).toEqual({ kind: "task", id: "tsk42" });
});

test("parseMarkdownLink: task: with empty target", () => {
  expect(parseMarkdownLink("task:")).toEqual({ kind: "empty" });
});

test("postprocessWikilinks: task link with matching label collapses to bare wikilink", () => {
  expect(postprocessWikilinks("see [tsk42](task:tsk42) here"))
    .toBe("see [[tsk42]] here");
});

test("postprocessWikilinks: task link with distinct label preserves the label", () => {
  expect(postprocessWikilinks("see [fix the parser](task:tsk42) here"))
    .toBe("see [[tsk42|fix the parser]] here");
});

// Broken links: a `[[…]]` target matching no known ref shape (e.g. the
// GitHub `[[#13]]` form) becomes an `oxplow-invalid:` link that the
// renderer shows as broken and non-clickable. Valid slugs — including
// the `.md` form — stay live.

test("preprocessWikilinks: GitHub-style [[#13]] becomes an oxplow-invalid link", () => {
  expect(preprocessWikilinks("Follow-up in [[#13]]."))
    .toBe("Follow-up in [#13](oxplow-invalid:%2313).");
});

test("preprocessWikilinks: valid slugs (incl .md) stay live wiki links", () => {
  expect(preprocessWikilinks("[[architecture]]")).toBe("[architecture](architecture)");
  expect(preprocessWikilinks("[[some_note-2]]")).toBe("[some_note-2](some_note-2)");
  expect(preprocessWikilinks("[[some-note.md]]")).toBe("[some-note.md](some-note.md)");
});

test("parseMarkdownLink: oxplow-invalid decodes the target into a broken link", () => {
  const parsed = parseMarkdownLink("oxplow-invalid:%2313");
  expect(parsed.kind).toBe("broken");
  if (parsed.kind === "broken") {
    expect(parsed.reason).toContain("#13");
    expect(parsed.reason).toContain("not a recognized reference");
  }
});

test("postprocessWikilinks: broken link round-trips back to the authored [[#13]]", () => {
  expect(postprocessWikilinks("Follow-up in [#13](oxplow-invalid:%2313).")).toBe(
    "Follow-up in [[#13]].",
  );
  // Full identity through both directions.
  expect(postprocessWikilinks(preprocessWikilinks("see [[#13]] please"))).toBe(
    "see [[#13]] please",
  );
});
