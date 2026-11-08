import { expect, test } from "bun:test";
import { linkRef, parseMarkdownLink, postprocessWikilinks, preprocessWikilinks, refLinkOf, relativeFilePath } from "./MarkdownView.js";
import {
  directoryRef,
  fileRef,
  gitCommitRef,
  workItemTabRef,
  wikiPageRef,
} from "../../tabs/pageRefs.js";
import { WORKING } from "../../revision.js";
import { commitUri, dirUri, fileLinesUri, fileUri, resourceUri, wikiUri, workItemUri } from "../../uri/uri.js";

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
  expect(parseMarkdownLink("oxplow:oxplow/file?path=src/foo.ts")).toEqual({ kind: "file", path: "src/foo.ts", version: null });
});

test("parseMarkdownLink: file: scheme with line suffix", () => {
  expect(parseMarkdownLink(fileLinesUri("src/foo.ts", 42, 42))).toEqual({ kind: "file", path: "src/foo.ts", line: 42, version: null });
});

test("parseMarkdownLink: file: scheme with @disk version", () => {
  expect(parseMarkdownLink(fileUri("src/foo.ts"))).toEqual({
    kind: "file",
    path: "src/foo.ts",
    version: null,
  });
});

test("parseMarkdownLink: file: scheme with @<sha> version", () => {
  expect(parseMarkdownLink(fileUri("src/foo.ts", "git:abc1234"))).toEqual({
    kind: "file",
    path: "src/foo.ts",
    version: "git:abc1234",
  });
});

test("parseMarkdownLink: file: scheme with @HEAD version + line", () => {
  expect(parseMarkdownLink(fileLinesUri("src/foo.ts", 42, 42, "git:HEAD"))).toEqual({
    kind: "file",
    path: "src/foo.ts",
    line: 42,
    version: "git:HEAD",
  });
});

test("parseMarkdownLink: file: scheme without a rev is the working tree", () => {
  expect(parseMarkdownLink("oxplow:oxplow/file?path=src/foo.ts")).toEqual({
    kind: "file",
    path: "src/foo.ts",
    version: null,
  });
});

test("parseMarkdownLink: file: scheme with empty target", () => {
  expect(parseMarkdownLink("oxplow:oxplow/file")).toEqual({ kind: "empty" });
});

test("preprocessWikilinks: rewrites file path target to file: link", () => {
  expect(preprocessWikilinks("see [[src/foo.ts]] for context"))
    .toBe(`see [src/foo.ts](${fileUri("src/foo.ts")}) for context`);
});

test("preprocessWikilinks: file path with line suffix", () => {
  expect(preprocessWikilinks("see [[src/foo.ts:88]]"))
    .toBe(`see [src/foo.ts:88](${fileLinesUri("src/foo.ts", 88, 88)})`);
});

test("preprocessWikilinks: file path with @version is preserved verbatim", () => {
  // The version becomes the file URI's `rev` (none for disk/local); the
  // click handler decodes it back into a Revision via parseMarkdownLink.
  expect(preprocessWikilinks("see [[src/foo.ts@HEAD]]"))
    .toBe(`see [src/foo.ts@HEAD](${fileUri("src/foo.ts", "git:HEAD")})`);
  expect(preprocessWikilinks("see [[src/foo.ts@disk:42]]"))
    .toBe(`see [src/foo.ts@disk:42](${fileLinesUri("src/foo.ts", 42, 42)})`);
});

test("preprocessWikilinks: |display syntax", () => {
  expect(preprocessWikilinks("the [[src/foo.ts|foo helper]] does X"))
    .toBe(`the [foo helper](${fileUri("src/foo.ts")}) does X`);
});

test("preprocessWikilinks: bare slug routes to wiki note", () => {
  expect(preprocessWikilinks("see [[architecture]] note"))
    .toBe("see [architecture](oxplow:oxplow/wiki?slug=architecture) note");
});

test("preprocessWikilinks: leaves wikilinks inside fenced code untouched", () => {
  const input = "Use this:\n```\n[[src/foo.ts]]\n```\nbut [[src/bar.ts]] is rewritten.";
  const out = preprocessWikilinks(input);
  expect(out).toContain("```\n[[src/foo.ts]]\n```");
  expect(out).toContain(`[src/bar.ts](${fileUri("src/bar.ts")})`);
});

test("preprocessWikilinks: leaves wikilinks inside inline backticks untouched", () => {
  expect(preprocessWikilinks("use the literal `[[path]]` syntax"))
    .toBe("use the literal `[[path]]` syntax");
});

test("preprocessWikilinks: handles multiple wikilinks on one line", () => {
  expect(preprocessWikilinks("[[src/a.ts]] and [[src/b.ts]] interact"))
    .toBe(`[src/a.ts](${fileUri("src/a.ts")}) and [src/b.ts](${fileUri("src/b.ts")}) interact`);
});

test("preprocessWikilinks: a target with no extension and no slash is a slug", () => {
  expect(preprocessWikilinks("[[architecture]]"))
    .toBe("[architecture](oxplow:oxplow/wiki?slug=architecture)");
});

test("preprocessWikilinks: nested-looking brackets do not match", () => {
  // Unmatched / pathological — just ensure we don't crash and don't rewrite
  // when target contains brackets.
  expect(preprocessWikilinks("[[ ]]"))
    .toBe("[[ ]]");
});

test("preprocessWikilinks: bare 7-char hex resolves to a commit link", () => {
  // The display text shrinks to the canonical short SHA so a 40-char raw
  // target doesn't blow up inline prose.
  expect(preprocessWikilinks("introduced in [[abc1234]]"))
    .toBe(`introduced in [abc1234](${commitUri("abc1234")})`);
});

test("preprocessWikilinks: full 40-char SHA renders short display, full target", () => {
  const sha = "0123456789abcdef0123456789abcdef01234567";
  expect(preprocessWikilinks(`see [[${sha}]]`))
    .toBe(`see [0123456](${commitUri(sha)})`);
});

test("preprocessWikilinks: explicit git: prefix", () => {
  expect(preprocessWikilinks("[[git:deadbeef]]"))
    .toBe(`[deadbee](${commitUri("deadbeef")})`);
});

test("preprocessWikilinks: |display label overrides short-sha shrinking", () => {
  expect(preprocessWikilinks("[[abc1234|the migration commit]]"))
    .toBe(`[the migration commit](${commitUri("abc1234")})`);
});

test("preprocessWikilinks: SHA detection is case-insensitive and normalizes to lowercase", () => {
  // Display text + href both normalize to lowercase so two notes spelling
  // the same sha differently render identically and hit the same tab.
  expect(preprocessWikilinks("[[ABC1234]]"))
    .toBe(`[abc1234](${commitUri("abc1234")})`);
});

test("preprocessWikilinks: 6-char hex is too short to be a SHA — treated as slug", () => {
  expect(preprocessWikilinks("[[abc123]]"))
    .toBe("[abc123](oxplow:oxplow/wiki?slug=abc123)");
});

test("preprocessWikilinks: hex with non-hex chars is a slug", () => {
  expect(preprocessWikilinks("[[abc-1234]]"))
    .toBe("[abc-1234](oxplow:oxplow/wiki?slug=abc-1234)");
});

test("parseMarkdownLink: commit: scheme", () => {
  expect(parseMarkdownLink("oxplow:oxplow/commit?sha=abc1234")).toEqual({ kind: "commit", sha: "abc1234" });
});

test("parseMarkdownLink: commit: with empty target", () => {
  expect(parseMarkdownLink("oxplow:oxplow/commit")).toEqual({ kind: "empty" });
});

test("parseMarkdownLink: dir: scheme strips trailing slash", () => {
  expect(parseMarkdownLink("oxplow:oxplow/dir?path=src/components")).toEqual({ kind: "directory", path: "src/components" });
  expect(parseMarkdownLink("oxplow:oxplow/dir?path=src/components/")).toEqual({ kind: "directory", path: "src/components" });
});

test("parseMarkdownLink: dir: with empty target", () => {
  expect(parseMarkdownLink("oxplow:oxplow/dir")).toEqual({ kind: "empty" });
});

test("preprocessWikilinks: dir: prefix rewrites to dir: href", () => {
  expect(preprocessWikilinks("see [[dir:src/components]] for the buttons"))
    .toBe(`see [src/components](${dirUri("src/components")}) for the buttons`);
});

test("preprocessWikilinks: dir: prefix tolerates trailing slash on the path", () => {
  expect(preprocessWikilinks("[[dir:src/components/]]"))
    .toBe(`[src/components](${dirUri("src/components")})`);
});

test("preprocessWikilinks: dir: prefix with custom display label", () => {
  expect(preprocessWikilinks("[[dir:src/components|the components folder]]"))
    .toBe(`[the components folder](${dirUri("src/components")})`);
});

// postprocessWikilinks is the inverse used on RichTextField → disk.
// It converts standard markdown links with our internal schemes back
// into `[[ ]]` wikilink form so wiki pages keep their authored shape.

test("postprocessWikilinks: file link with matching label collapses to bare wikilink", () => {
  expect(postprocessWikilinks(`see [src/foo.ts](${fileUri("src/foo.ts")}) here`))
    .toBe("see [[src/foo.ts]] here");
});

test("postprocessWikilinks: file link with distinct label preserves the label", () => {
  expect(postprocessWikilinks(`see [the helper](${fileUri("src/foo.ts")}) here`))
    .toBe("see [[src/foo.ts|the helper]] here");
});

test("postprocessWikilinks: dir link", () => {
  expect(postprocessWikilinks(`[src/components](${dirUri("src/components")})`))
    .toBe("[[dir:src/components]]");
  expect(postprocessWikilinks(`[the components folder](${dirUri("src/components")})`))
    .toBe("[[dir:src/components|the components folder]]");
});

test("postprocessWikilinks: gitcommit link with short-sha label drops the label", () => {
  expect(postprocessWikilinks(`see [abc1234](${commitUri("abc1234deadbeef")}) here`))
    .toBe("see [[git:abc1234deadbeef]] here");
});

test("postprocessWikilinks: gitcommit link with custom label preserves it", () => {
  expect(postprocessWikilinks(`see [the fix](${commitUri("abc1234deadbeef")}) here`))
    .toBe("see [[git:abc1234deadbeef|the fix]] here");
});

test("postprocessWikilinks: leaves plain http/https/internal-slug links alone", () => {
  expect(postprocessWikilinks("see [docs](https://example.com) plus [arch](architecture)"))
    .toBe("see [docs](https://example.com) plus [arch](architecture)");
});

test("postprocessWikilinks: skips image links and inline code", () => {
  const src = `![alt](${fileUri("foo.png")}) and \`[label](${fileUri("foo.ts")})\` literal`;
  expect(postprocessWikilinks(src)).toBe(src);
});

test("postprocessWikilinks: skips fenced code blocks", () => {
  const src = `before\n\`\`\`\n[label](${fileUri("foo.ts")})\n\`\`\`\nafter [label](${fileUri("foo.ts")})`;
  expect(postprocessWikilinks(src))
    .toBe(`before\n\`\`\`\n[label](${fileUri("foo.ts")})\n\`\`\`\nafter [[foo.ts|label]]`);
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
    `see [[${workItemUri("issues", "ENG-1")}]] for theirs`,
  ];
  const ids = (id: string) => (/^tsk\d+$/.test(id) ? workItemUri("oxplow", id) : null);
  for (const sample of samples) {
    expect(postprocessWikilinks(preprocessWikilinks(sample, ids))).toBe(sample);
  }
});

// Work item wikilinks: a bare id of the active list's (`[[tsk42]]` with
// oxplow's tasks, by its declared id pattern) or a full ref → the item's
// `oxplow:oxplow/work_item?id=<id>&provider=<provider>` href; the renderer swaps the
// `tsk<id>` token for the task title at display time. The backend ref
// extractor (refs.rs) already recognizes the same form for backlinks.

const oxplowIds = (id: string) => (/^tsk\d+$/.test(id) ? workItemUri("oxplow", id) : null);

test("preprocessWikilinks: an id of the active list rewrites to its work_item href", () => {
  expect(preprocessWikilinks("see [[tsk42]] for context", oxplowIds))
    .toBe(`see [tsk42](${workItemUri("oxplow", "tsk42")}) for context`);
});

test("preprocessWikilinks: a work item with a custom display label", () => {
  expect(preprocessWikilinks("[[tsk42|fix the parser]]", oxplowIds))
    .toBe(`[fix the parser](${workItemUri("oxplow", "tsk42")})`);
});

test("preprocessWikilinks: a full work item ref links whichever list", () => {
  const eng1 = workItemUri("issues", "ENG-1");
  expect(preprocessWikilinks(`[[${eng1}]]`)).toBe(`[${eng1}](${eng1})`);
});

test("preprocessWikilinks: what isn't the active list's id is a wiki slug", () => {
  // `tsk-notes` doesn't match the list's ids; with no list (none), even
  // `tsk42` is an ordinary slug.
  expect(preprocessWikilinks("[[tsk-notes]]", oxplowIds)).toBe("[tsk-notes](oxplow:oxplow/wiki?slug=tsk-notes)");
  expect(preprocessWikilinks("[[tsk42]]")).toBe("[tsk42](oxplow:oxplow/wiki?slug=tsk42)");
});

test("parseMarkdownLink: work_item: scheme, any list", () => {
  expect(parseMarkdownLink("oxplow:oxplow/work_item?id=tsk42&provider=oxplow")).toEqual({ kind: "work_item", ref: "oxplow:oxplow/work_item?id=tsk42&provider=oxplow" });
  expect(parseMarkdownLink("oxplow:oxplow/work_item?id=ENG-1&provider=issues")).toEqual({ kind: "work_item", ref: "oxplow:oxplow/work_item?id=ENG-1&provider=issues" });
});

test("parseMarkdownLink: work_item: with empty target", () => {
  expect(parseMarkdownLink("oxplow:oxplow/work_item")).toEqual({ kind: "empty" });
});

test("postprocessWikilinks: task link with matching label collapses to bare wikilink", () => {
  expect(postprocessWikilinks(`see [tsk42](${workItemUri("oxplow", "tsk42")}) here`))
    .toBe("see [[tsk42]] here");
});

test("postprocessWikilinks: task link with distinct label preserves the label", () => {
  expect(postprocessWikilinks(`see [fix the parser](${workItemUri("oxplow", "tsk42")}) here`))
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

test("a wiki page's link goes into the editor as its URI and comes back as [[slug]]", () => {
  for (const body of ["See [[architecture]].", "See [[architecture|the design]]."]) {
    expect(postprocessWikilinks(preprocessWikilinks(body))).toBe(body);
  }
});

test("preprocessWikilinks: valid slugs (incl .md) stay live wiki links", () => {
  expect(preprocessWikilinks("[[architecture]]")).toBe("[architecture](oxplow:oxplow/wiki?slug=architecture)");
  expect(preprocessWikilinks("[[some_note-2]]")).toBe("[some_note-2](oxplow:oxplow/wiki?slug=some_note-2)");
  expect(preprocessWikilinks("[[some-note.md]]")).toBe("[some-note.md](oxplow:oxplow/wiki?slug=some-note)");
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

// Every internal link kind has to resolve to a tab the same way, or
// "open in a new tab" works on some and silently does nothing on
// others — which is what tsk265 found: task links honoured the caller's
// newTab, file / directory / commit links fell through to legacy
// single-tab callbacks and dropped it.

test("linkRef: a wiki link targets its page", () => {
  expect(linkRef({ kind: "internal", slug: "some-page" })).toBe(wikiPageRef("some-page").id);
});

test("linkRef: a work item link targets its page", () => {
  expect(linkRef({ kind: "work_item", ref: "oxplow:oxplow/work_item?id=tsk42&provider=oxplow" })).toBe("oxplow:oxplow/work_item?id=tsk42&provider=oxplow");
});

test("linkRef: a directory link targets its directory", () => {
  expect(linkRef({ kind: "directory", path: "src/components" })).toBe(
    directoryRef("src/components").id,
  );
});

test("linkRef: a commit link targets its commit", () => {
  expect(linkRef({ kind: "commit", sha: "abc123" })).toBe(gitCommitRef("abc123").id);
});

// A bare file wikilink means "the working tree"; one that pinned a
// version must keep it rather than being silently substituted.
test("linkRef: a bare file link coerces to the working tree", () => {
  expect(linkRef({ kind: "file", path: "src/main.rs" })).toBe(fileRef("src/main.rs", WORKING).id);
});

test("linkRef: a pinned file link keeps the version it was authored with", () => {
  const version = "git:deadbeef" as const;
  expect(linkRef({ kind: "file", path: "src/main.rs", version })).toBe(
    fileRef("src/main.rs", version).id,
  );
});

test("linkRef: things that aren't pages have no target", () => {
  expect(linkRef({ kind: "external" })).toBeNull();
  expect(linkRef({ kind: "anchor" })).toBeNull();
  expect(linkRef({ kind: "empty" })).toBeNull();
});

// A URI put in as a link (completion, a drop, a paste) saves as the
// wikilink a person would have typed, and a ref with no bare form keeps
// its title as the label.
test("refLinkOf: each kind saves as its bare wikilink", () => {
  const saved = (uri: string, title = "T", mention?: (id: string) => string | null) => {
    const { href, text } = refLinkOf(uri, title, mention);
    return postprocessWikilinks(`[${text}](${href})`);
  };
  expect(saved(wikiUri("local-snapshots"))).toBe("[[local-snapshots]]");
  expect(saved(fileUri("src/a.ts"))).toBe("[[src/a.ts]]");
  expect(saved(fileLinesUri("src/a.ts", 12, 12))).toBe("[[src/a.ts:12]]");
  expect(saved(dirUri("src/components"))).toBe("[[dir:src/components]]");
  const sha = "0123456789abcdef0123456789abcdef01234567";
  expect(saved(commitUri(sha))).toBe(`[[git:${sha}]]`);
  const item = workItemUri("oxplow", "tsk42");
  // A bare id when it is the active list's, else the whole URI.
  expect(saved(item, "T", (id) => (id === "tsk42" ? item : null))).toBe("[[tsk42]]");
  expect(saved(item, "T", () => null)).toBe(`[[${item}]]`);
  expect(saved(resourceUri("acme/pr", { id: 12 }), "Fix the parser")).toBe(`[Fix the parser](${resourceUri("acme/pr", { id: 12 })})`);
});

// A markdown file's relative links and images are its folder's: given the
// file's URI as `base`, `docs/guide.md` is that file (on the base's
// filesystem and revision), not a wiki page; `../x.png` climbs; `/top.md`
// is the root's. An anchor, a web link and an oxplow URI read as before,
// and with no base a relative `.md` link is a wiki slug, as the wiki's are.
test("parseMarkdownLink: a relative link resolves against the base file's folder", () => {
  const readme = "oxplow:oxplow/file?path=README.md";
  expect(parseMarkdownLink("docs/guide.md", readme)).toEqual({ kind: "file", path: "docs/guide.md", version: null, filesystem: null });
  expect(parseMarkdownLink("./docs/guide.md#install", readme)).toEqual({ kind: "file", path: "docs/guide.md", version: null, filesystem: null });
  const deep = "oxplow:oxplow/file?path=a/b/README.md&rev=git:abc1234&filesystem=oxplow:extensions";
  expect(parseMarkdownLink("../x.png?raw=1", deep)).toEqual({ kind: "file", path: "a/x.png", version: "git:abc1234", filesystem: "oxplow:extensions" });
  expect(parseMarkdownLink("/top.md", deep)).toEqual({ kind: "file", path: "top.md", version: "git:abc1234", filesystem: "oxplow:extensions" });
  expect(parseMarkdownLink("sibling%20file.md", deep)).toEqual({ kind: "file", path: "a/b/sibling file.md", version: "git:abc1234", filesystem: "oxplow:extensions" });
  expect(parseMarkdownLink("#install", readme)).toEqual({ kind: "anchor" });
  expect(parseMarkdownLink("https://a.test/", readme)).toEqual({ kind: "external" });
  expect(parseMarkdownLink("oxplow:oxplow/wiki?slug=notes", readme)).toEqual({ kind: "internal", slug: "notes" });
  expect(parseMarkdownLink("docs/guide.md")).toEqual({ kind: "internal", slug: "docs/guide" });
});

// A document drawn by a chosen page (its base's `#oxplow.renderer=`) links
// to a sibling of the same kind as itself: `DEV.md` from a README read as
// a document opens as a document, where `src/main.rs` opens as what it is.
test("parseMarkdownLink: a link to a file of the base's kind carries the page that draws the base", () => {
  const readme = "oxplow:oxplow/file?path=README.md&rev=git:abc1234#oxplow.renderer=oxplow/markdown";
  expect(parseMarkdownLink("DEV.md", readme)).toEqual({ kind: "file", path: "DEV.md", version: "git:abc1234", filesystem: null, renderer: "oxplow/markdown" });
  expect(parseMarkdownLink("docs/Guide.MD#top", readme)).toEqual({ kind: "file", path: "docs/Guide.MD", version: "git:abc1234", filesystem: null, renderer: "oxplow/markdown" });
  expect(parseMarkdownLink("src/main.rs", readme)).toEqual({ kind: "file", path: "src/main.rs", version: "git:abc1234", filesystem: null });
  expect(linkRef(parseMarkdownLink("DEV.md", readme))).toBe("oxplow:oxplow/file?path=DEV.md&rev=git:abc1234#oxplow.renderer=oxplow/markdown");
  expect(linkRef(parseMarkdownLink("src/main.rs", readme))).toBe("oxplow:oxplow/file?path=src/main.rs&rev=git:abc1234");
});

test("relativeFilePath: joins and climbs under the base file's folder, never above the root", () => {
  expect(relativeFilePath("a/b/README.md", "c.md")).toBe("a/b/c.md");
  expect(relativeFilePath("a/b/README.md", "../../../c.md")).toBe("c.md");
  expect(relativeFilePath("README.md", "./img/./logo.png")).toBe("img/logo.png");
  expect(relativeFilePath("a/README.md", "/x/y.md")).toBe("x/y.md");
});
