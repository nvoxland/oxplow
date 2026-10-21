import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import { dedupeEntriesByTarget, edgeToInboundEntry, edgeToOutboundEntry, graphUriOf, humanRefTypeForTest } from "./useBacklinks.js";
import type { BacklinkEntry } from "./backlinkTypes.js";
import type { BacklinkEdge } from "../api.js";
import { setResources, type Resource } from "../resources.js";
import { collectorUri, commitUri, dirUri, effortUri, fileUri, lensUri, pageUri, resourceUri, wikiUri, workItemUri } from "../uri/uri.js";
import {
  directoryRef,
  effortDiffRef,
  fileRef,
  gitCommitRef,
  lensRef,
  wikiPageRef,
  pageTabRef,
  workItemTabRef,
} from "./pageRefs.js";

describe("graphUriOf", () => {
  // page_ref rows hang on a resource's URI (.context/resources.md), whichever list.
  test("a work item is its own URI", () => {
    const item = workItemUri("fake", "W-1");
    expect(graphUriOf(workItemTabRef(item))).toBe(item);
  });

  test("a wiki page, a file and a directory are theirs, a file without its revision", () => {
    expect(graphUriOf(wikiPageRef("url-schemes"))).toBe(wikiUri("url-schemes"));
    expect(graphUriOf(fileRef("src/app.ts"))).toBe(fileUri("src/app.ts"));
    expect(graphUriOf(directoryRef("src/components"))).toBe(dirUri("src/components"));
  });

  test("gitCommitRef is the commit's", () => {
    expect(graphUriOf(gitCommitRef("abc1234"))).toBe(commitUri("abc1234"));
  });

  test("any other kind with a page is its own URI, without its view params", () => {
    expect(graphUriOf(lensRef("oxplow-bundled/findings", { range: "7d" }))).toBe(lensUri("oxplow-bundled/findings"));
    expect(graphUriOf(effortDiffRef("eff12"))).toBe(effortUri("eff12"));
  });

  test("a shell route has none", () => {
    expect(graphUriOf(pageTabRef("settings"))).toBeNull();
  });
});

describe("humanRefType", () => {
  test("body-mention ref_types all collapse to 'mention'", () => {
    for (const rt of [
      "wikilink",
      "wiki_file_ref",
      "wiki_dir_ref",
      "summary_wikilink",
      "summary_file_ref",
      "summary_dir_ref",
      "work_item_mention",
      "summary_work_item_mention",
      "finding_mention",
      "summary_finding_mention",
      "commit_mention",
      "summary_commit_mention",
    ]) {
      expect(humanRefTypeForTest(rt, null)).toBe("mention");
    }
  });

  // A work item's comments' mentions are the item's own edges: the
  // backlink says it's in a comment.
  test("comment-mention ref_types read as 'comment'", () => {
    for (const rt of [
      "comment_file_ref",
      "comment_dir_ref",
      "comment_wikilink",
      "comment_work_item_mention",
      "comment_finding_mention",
      "comment_commit_mention",
    ]) {
      expect(humanRefTypeForTest(rt, null)).toBe("comment");
    }
  });

  test("touched_file derives action from change_kind", () => {
    expect(humanRefTypeForTest("touched_file", JSON.stringify({ change_kind: "created" }))).toBe("created");
    // `updated` normalizes to "modified"
    expect(humanRefTypeForTest("touched_file", JSON.stringify({ change_kind: "updated" }))).toBe("modified");
    expect(humanRefTypeForTest("touched_file", JSON.stringify({ change_kind: "deleted" }))).toBe("deleted");
    // Legacy rows with no extra fall back to "modified" (the dominant case)
    expect(humanRefTypeForTest("touched_file", null)).toBe("modified");
  });

  test("impact surfaces the declared action verb without an 'impact' prefix", () => {
    expect(humanRefTypeForTest("impact", JSON.stringify({ action: "created" }))).toBe("created");
    expect(humanRefTypeForTest("impact", JSON.stringify({ action: "updated" }))).toBe("modified");
    expect(humanRefTypeForTest("impact", JSON.stringify({ action: "resolved" }))).toBe("resolved");
    expect(humanRefTypeForTest("impact", JSON.stringify({ action: "referenced" }))).toBe("referenced");
    expect(humanRefTypeForTest("impact", null)).toBe("impact");
  });

  test("typed task links humanize the sub-type", () => {
    expect(humanRefTypeForTest("work_item_link:blocks", null)).toBe("blocks");
    expect(humanRefTypeForTest("work_item_link:relates_to", null)).toBe("relates to");
    expect(humanRefTypeForTest("work_item_link:discovered_from", null)).toBe("discovered from");
  });

  test("finding_path stays as 'found in'", () => {
    expect(humanRefTypeForTest("finding_path", null)).toBe("found in");
  });

  test("unknown ref_types fall through to the raw string", () => {
    expect(humanRefTypeForTest("future_thing", null)).toBe("future_thing");
  });
});

describe("dedupeEntriesByTarget", () => {
  test("collapses wiki + on-disk file shadow into a single wiki row", () => {
    const entries: BacklinkEntry[] = [
      { open: "oxplow:oxplow/file?path=.oxplow/wiki/local-snapshots.md", target: "oxplow:oxplow/file?path=.oxplow/wiki/local-snapshots.md", label: ".oxplow/wiki/local-snapshots.md", subtitle: "modified" },
      { open: "oxplow:oxplow/wiki?slug=local-snapshots", target: "oxplow:oxplow/wiki?slug=local-snapshots", label: "Local Snapshots", subtitle: "created" },
      { open: "oxplow:oxplow/wiki?slug=local-snapshots", target: "oxplow:oxplow/wiki?slug=local-snapshots", label: "Local Snapshots", subtitle: "mention" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(1);
    expect(out[0].open).toBe("oxplow:oxplow/wiki?slug=local-snapshots");
    expect(out[0].label).toBe("Local Snapshots");
    // Subtitles merged in first-seen order, deduped.
    expect(out[0].subtitle).toBe("modified · created · mention");
  });

  test("preserves rows that point at different targets", () => {
    const entries: BacklinkEntry[] = [
      { open: "oxplow:oxplow/wiki?slug=a", target: "oxplow:oxplow/wiki?slug=a", label: "A", subtitle: "mention" },
      { open: "oxplow:oxplow/wiki?slug=b", target: "oxplow:oxplow/wiki?slug=b", label: "B", subtitle: "mention" },
      { open: "oxplow:oxplow/file?path=src/x.ts", target: "oxplow:oxplow/file?path=src/x.ts", label: "src/x.ts", subtitle: "modified" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(3);
  });

  test("collapses two ref_types pointing at the same wiki page", () => {
    const entries: BacklinkEntry[] = [
      { open: "oxplow:oxplow/wiki?slug=url-schemes", target: "oxplow:oxplow/wiki?slug=url-schemes", label: "URL Schemes", subtitle: "created" },
      { open: "oxplow:oxplow/wiki?slug=url-schemes", target: "oxplow:oxplow/wiki?slug=url-schemes", label: "URL Schemes", subtitle: "mention" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(1);
    expect(out[0].subtitle).toBe("created · mention");
  });

  test("file path that doesn't look like a wiki shadow stays a file row", () => {
    const entries: BacklinkEntry[] = [
      { open: "oxplow:oxplow/file?path=src/foo.rs", target: "oxplow:oxplow/file?path=src/foo.rs", label: "src/foo.rs", subtitle: "modified" },
      { open: "oxplow:oxplow/file?path=src/foo.rs", target: "oxplow:oxplow/file?path=src/foo.rs", label: "src/foo.rs", subtitle: "mention" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(1);
    expect(out[0].open).toBe("oxplow:oxplow/file?path=src/foo.rs");
    expect(out[0].subtitle).toBe("modified · mention");
  });

  test("drops blank subtitles cleanly", () => {
    const entries: BacklinkEntry[] = [
      { open: "oxplow:oxplow/wiki?slug=a", target: "oxplow:oxplow/wiki?slug=a", label: "A", subtitle: "" },
      { open: "oxplow:oxplow/wiki?slug=a", target: "oxplow:oxplow/wiki?slug=a", label: "A", subtitle: "mention" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out[0].subtitle).toBe("mention");
  });
});

describe("edges to any ref", () => {
  // The kinds are read: core's, as the registry lists them.
  const coreKind = (name: string, page: string | null): Resource => ({
    resource: `oxplow/${name}`,
    extension: "oxplow",
    label: name,
    params: { id: { required: true, pattern: null } },
    additional: false,
    resolve: null,
    title: null,
    searchable: null,
    icon: null,
    wikilink: null,
    page,
  });
  const CORE = () => [coreKind("effort", null), coreKind("work_item", null), coreKind("collector", null)];
  beforeEach(() => setResources(CORE()));
  afterEach(() => setResources([]));
  const edge = (over: Partial<BacklinkEdge>): BacklinkEdge => ({
    source: workItemUri("oxplow", "tsk1"),
    target: wikiUri("x"),
    ref_type: "work_item_link:about",
    source_extra: null,
    source_label: null,
    resolved: true,
    ...over,
  });

  test("an about link to a collector resolves and opens a page", () => {
    const entry = edgeToOutboundEntry(
      edge({ target: collectorUri("triage", "todos"), source_label: "Triage todos" }),
    );
    expect(entry.dead).toBe(false);
    expect(entry.open).toBe(pageUri("settings"));
    expect(entry.label).toBe("Triage todos");
    expect(entry.subtitle).toBe("about");
    expect(entry.linkType).toBe("about");
    expect(entry.target).toBe(collectorUri("triage", "todos"));
  });

  test("a link to a core kind with a page opens it", () => {
    const entry = edgeToOutboundEntry(edge({ target: effortUri("eff12"), source_label: "An effort" }));
    expect(entry.open).toBe(effortUri("eff12"));
  });

  test("a link to an extension's resource opens the page its registry row names", () => {
    setResources([
      {
        resource: "acme/pr",
        extension: "acme",
        label: "PR",
        params: { id: { required: true, pattern: null } },
        additional: false,
        resolve: null,
        title: null,
        searchable: null,
        icon: null,
        wikilink: null,
        page: pageUri("acme/pr"),
      },
    ]);
    try {
      const entry = edgeToOutboundEntry(edge({ target: resourceUri("acme/pr", { id: 12 }), source_label: "PR 12" }));
      expect(entry.open).toBe(resourceUri("acme/pr", { id: 12 }));
      expect(entry.dead).toBe(false);
    } finally {
      setResources(CORE());
    }
  });

  test("a link to an uninstalled extension's resource is dead, with its raw text, and does not throw", () => {
    const entry = edgeToOutboundEntry(
      edge({ target: resourceUri("oxplow/acme_pr", { id: 12 }), resolved: false, source_label: null }),
    );
    expect(entry.open).toBeNull();
    expect(entry.dead).toBe(true);
    expect(entry.label).toBe(resourceUri("oxplow/acme_pr", { id: 12 }));
    expect(dedupeEntriesByTarget([entry, entry])).toHaveLength(1);
  });

  test("an inbound edge reads the source side", () => {
    const entry = edgeToInboundEntry(edge({ source_label: "Fix it" }));
    expect(entry.open).toBe("oxplow:oxplow/work_item?id=tsk1&provider=oxplow");
    expect(entry.label).toBe("Fix it");
  });
});
