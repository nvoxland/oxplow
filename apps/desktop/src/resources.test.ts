import { afterEach, describe, expect, test } from "bun:test";
import { GitPullRequest } from "lucide-react";

import { linkRef, parseMarkdownLink, preprocessWikilinks } from "./components/Markdown/MarkdownView.js";
import { pageKindIconComponent, pageKindLabel } from "./pageKinds.js";
import { extensionWikilinkRef, resourcesFromResult, setResources, type Resource } from "./resources.js";
import { refFromTabId } from "./tabs/pageRefs.js";
import { refTargetNow } from "./tabs/refOpener.js";
import { effortUri, resourceUri } from "./uri/uri.js";

const ACME_PR: Resource = {
  resource: "acme/pr",
  extension: "acme",
  label: "Pull request",
  params: { id: { required: true, pattern: "^\\d+$" } },
  additional: false,
  resolve: "v_acme_prs",
  title: null,
  searchable: null,
  icon: "git-pull-request",
  wikilink: { prefix: "pr", param: "id" },
};

const ACME_PR_URI = resourceUri("acme/pr", { id: 12 });

afterEach(() => setResources([]));

describe("an extension's ref kind (P8.D7)", () => {
  test("draws with its declared icon and label", () => {
    expect(pageKindIconComponent("acme/pr")).toBeNull();
    setResources([ACME_PR]);
    expect(pageKindIconComponent("acme/pr")).toBe(GitPullRequest);
    expect(pageKindLabel("acme/pr")).toBe("Pull request");
  });

  test("opens as itself, whether or not it is installed", () => {
    // Intent moved: the resource no longer names a page. Every oxplow URI is a
    // tab whose id is the URI; the renderer picked for it draws it.
    expect(refFromTabId(ACME_PR_URI)).toEqual({ id: ACME_PR_URI });
    setResources([ACME_PR]);
    expect(refFromTabId(ACME_PR_URI)).toEqual({ id: ACME_PR_URI });
    expect(refTargetNow(ACME_PR_URI)).toEqual({ tab: { id: ACME_PR_URI } });
  });

  test("a wikilink names one by its prefix, while installed", () => {
    setResources([ACME_PR]);
    expect(extensionWikilinkRef("pr:12")).toBe(ACME_PR_URI);
    expect(extensionWikilinkRef("pr:twelve")).toBeNull();
    const body = preprocessWikilinks("see [[pr:12]]");
    expect(body).toBe(`see [pr:12](${ACME_PR_URI})`);
    const parsed = parseMarkdownLink(ACME_PR_URI);
    expect(parsed).toEqual({ kind: "ref", ref: ACME_PR_URI });
    expect(linkRef(parsed)).toBe(ACME_PR_URI);
    setResources([]);
    expect(extensionWikilinkRef("pr:12")).toBeNull();
    expect(preprocessWikilinks("see [[pr:12]]")).toContain("oxplow-invalid:");
  });

  test("a sugar is read before a resource's own name, as the backend reads it", () => {
    const effort: Resource = {
      ...ACME_PR,
      resource: "oxplow/effort",
      extension: "oxplow",
      params: { id: { required: true, pattern: null } },
      wikilink: null,
    };
    // Without a declared prefix, an oxplow resource with one required param
    // is named by its own name.
    setResources([effort]);
    expect(extensionWikilinkRef("effort:eff3")).toBe(effortUri("eff3"));
    // An extension that declares the same prefix wins.
    setResources([effort, { ...ACME_PR, wikilink: { prefix: "effort", param: "id" } }]);
    expect(extensionWikilinkRef("effort:12")).toBe(ACME_PR_URI);
  });

  test("a long id matches as the backend's does (tsk917)", () => {
    setResources([ACME_PR]);
    expect(extensionWikilinkRef(`pr:${"1".repeat(300)}`)).toBe(resourceUri("acme/pr", { id: "1".repeat(300) }));
  });

  test("every character the backend spells is itself here (tsk917)", () => {
    // The backend's spelling: these escaped, everything else as is.
    const spell = (c: string) => ("\\.+*?()|[]{}^$-&~#".includes(c) ? `\\${c}` : c);
    const printable = Array.from({ length: 95 }, (_, i) => String.fromCharCode(32 + i));
    for (const c of printable) {
      for (const pattern of [`^${spell(c)}$`, `^[${spell(c)}a]$`]) {
        const re = new RegExp(pattern);
        for (const d of printable) {
          expect([c, d, re.test(d)]).toEqual([c, d, d === c || (pattern.includes("a]") && d === "a")]);
        }
      }
    }
  });

  test("reads v_resource rows", () => {
    const kinds = resourcesFromResult({
      columns: ["resource", "extension", "label", "params", "additional", "resolve", "title", "searchable", "icon", "wikilink"],
      rows: [
        [
          "acme/pr",
          "acme",
          "Pull request",
          '{"id":{"required":true,"pattern":"^\\\\d+$"}}',
          0,
          "v_acme_prs",
          null,
          null,
          "git-pull-request",
          '{"prefix":"pr","param":"id"}',
        ],
      ],
      truncated: false,
      reads: { models: [], tables: [] },
    } as never);
    expect(kinds).toEqual([ACME_PR]);
  });
});

// oxplow's own resources come in the same list, with no wikilink or icon.
test("a core resource's row reads as one", () => {
  const kinds = resourcesFromResult({
    columns: ["resource", "extension", "label", "params", "additional", "resolve", "title", "searchable", "icon", "wikilink"],
    rows: [["oxplow/handoff", "oxplow", "Handoff", '{"id":{"required":true,"pattern":"^hnd\\\\d+$"}}', 0, "v_handoff", null, null, null, null]],
    truncated: false,
    reads: { models: [], tables: [] },
  } as never);
  expect(kinds[0]).toMatchObject({
    resource: "oxplow/handoff",
    extension: "oxplow",
    label: "Handoff",
    params: { id: { required: true, pattern: "^hnd\\d+$" } },
    additional: false,
    icon: null,
    wikilink: null,
  });
});
