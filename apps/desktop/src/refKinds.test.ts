import { afterEach, describe, expect, test } from "bun:test";
import { GitPullRequest } from "lucide-react";

import { linkTarget, parseMarkdownLink, preprocessWikilinks } from "./components/Wiki/MarkdownView.js";
import { pageKindIconComponent, pageKindLabel } from "./pageKinds.js";
import { pluginWikilinkRef, refKindsFromResult, setRefKinds, type RefKind } from "./refKinds.js";
import { refFromTabId } from "./tabs/pageRefs.js";

const ACME_PR: RefKind = {
  kind: "acme_pr",
  extension: "acme",
  label: "Pull request",
  idPattern: "^\\d+$",
  wikilinks: ["pr"],
  resolve: "v_acme_prs",
  page: "page:ext.acme.pr",
  icon: "git-pull-request",
};

afterEach(() => setRefKinds([]));

describe("an extension's ref kind (P8.D7)", () => {
  test("draws with its declared icon and label", () => {
    expect(pageKindIconComponent("acme_pr")).toBeNull();
    setRefKinds([ACME_PR]);
    expect(pageKindIconComponent("acme_pr")).toBe(GitPullRequest);
    expect(pageKindLabel("acme_pr")).toBe("Pull request");
  });

  test("opens its extension's page with the ref", () => {
    expect(refFromTabId("acme_pr:12")).toBeNull();
    setRefKinds([ACME_PR]);
    const tab = refFromTabId("acme_pr:12");
    expect(tab).toEqual({
      id: "page:ext.acme.pr?ref=acme_pr:12",
      kind: "ext-page",
      payload: { extension: "acme", page: "pr", params: { ref: "acme_pr:12" } },
    });
    // The page id alone (a restored tab) carries the ref back.
    expect(refFromTabId(tab!.id)).toEqual(tab);
  });

  test("a wikilink names one by its kind or its prefix, while installed", () => {
    setRefKinds([ACME_PR]);
    expect(pluginWikilinkRef("pr:12")).toBe("acme_pr:12");
    expect(pluginWikilinkRef("acme_pr:12")).toBe("acme_pr:12");
    expect(pluginWikilinkRef("pr:twelve")).toBeNull();
    const body = preprocessWikilinks("see [[pr:12]]");
    expect(body).toBe("see [pr:12](acme_pr:12)");
    const parsed = parseMarkdownLink("acme_pr:12");
    expect(parsed).toEqual({ kind: "ref", ref: "acme_pr:12" });
    expect(linkTarget(parsed)?.id).toBe("page:ext.acme.pr?ref=acme_pr:12");
    setRefKinds([]);
    expect(pluginWikilinkRef("pr:12")).toBeNull();
    expect(preprocessWikilinks("see [[pr:12]]")).toContain("oxplow-invalid:");
  });

  test("reads v_ref_kind rows", () => {
    const kinds = refKindsFromResult({
      columns: ["kind", "extension", "label", "id_pattern", "wikilinks", "resolve", "page", "icon"],
      rows: [["acme_pr", "acme", "Pull request", "^\\d+$", '["pr"]', "v_acme_prs", "page:ext.acme.pr", "git-pull-request"]],
      truncated: false,
      reads: { models: [], tables: [] },
    } as never);
    expect(kinds).toEqual([ACME_PR]);
  });
});
