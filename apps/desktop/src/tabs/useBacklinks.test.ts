import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import { canonicalIdForTarget, dedupeEntriesByTarget, edgeToInboundEntry, edgeToOutboundEntry, humanRefTypeForTest } from "./useBacklinks.js";
import type { BacklinkEntry } from "./backlinkTypes.js";
import type { BacklinkEdge } from "../api.js";
import { setResources, type Resource } from "../resources.js";
import {
  directoryRef,
  effortDiffRef,
  fileRef,
  gitCommitRef,
  lensRef,
  wikiPageRef,
  workItemTabRef,
} from "./pageRefs.js";

describe("canonicalIdForTarget", () => {
  // page_ref rows are a canonical ref's (kind, id): a work item is
  // `work_item` / `<provider>:<id>` (.context/resources.md), whichever list.
  test("a work item is its ref's id, as the graph stores it", () => {
    expect(canonicalIdForTarget(workItemTabRef("oxplow:oxplow/work_item?id=tsk42&provider=oxplow"))).toBe("oxplow:tsk42");
    expect(canonicalIdForTarget(workItemTabRef("oxplow:oxplow/work_item?id=W-1&provider=fake"))).toBe("fake:W-1");
  });

  test("wikiPageRef returns slug", () => {
    expect(canonicalIdForTarget(wikiPageRef("url-schemes"))).toBe("url-schemes");
  });

  test("fileRef returns the workspace-relative path", () => {
    expect(canonicalIdForTarget(fileRef("src/app.ts"))).toBe("src/app.ts");
  });

  test("directoryRef returns the path", () => {
    expect(canonicalIdForTarget(directoryRef("src/components"))).toBe("src/components");
  });

  test("gitCommitRef returns the sha", () => {
    expect(canonicalIdForTarget(gitCommitRef("abc1234"))).toBe("abc1234");
  });

  test("any other kind with a page is its canonical ref's id", () => {
    expect(canonicalIdForTarget(lensRef("oxplow-bundled/findings"))).toBe("oxplow-bundled/findings");
    expect(canonicalIdForTarget(effortDiffRef("eff12"))).toBe("eff12");
  });

  test("untracked kinds return null", () => {
    expect(canonicalIdForTarget({ id: "settings", kind: "settings", payload: null })).toBeNull();
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
  const coreKind = (kind: string, page: string | null): Resource => ({
    kind, extension: null, label: null, idPattern: ".*", wikilinks: [], resolve: null, page, icon: null,
  });
  beforeEach(() => setResources([coreKind("effort", "{ref}"), coreKind("work_item", "{ref}"), coreKind("collector", null)]));
  afterEach(() => setResources([]));
  const edge = (over: Partial<BacklinkEdge>): BacklinkEdge => ({
    source_kind: "work_item",
    source_id: "oxplow:tsk1",
    target_kind: "wiki",
    target_id: "x",
    ref_type: "work_item_link:about",
    source_extra: null,
    source_label: null,
    resolved: true,
    ...over,
  });

  test("an about link to a collector resolves and opens a page", () => {
    const entry = edgeToOutboundEntry(
      edge({ target_kind: "collector", target_id: "triage/todos", source_label: "Triage todos" }),
    );
    expect(entry.dead).toBe(false);
    expect(entry.open).toBe("oxplow:oxplow/page?page=settings");
    expect(entry.label).toBe("Triage todos");
    expect(entry.subtitle).toBe("about");
    expect(entry.linkType).toBe("about");
    expect(entry.target).toBe("oxplow:oxplow/collector?collector=triage/todos");
  });

  test("a link to a core kind with a page opens it", () => {
    const entry = edgeToOutboundEntry(edge({ target_kind: "effort", target_id: "eff12", source_label: "An effort" }));
    expect(entry.open).toBe("oxplow:oxplow/effort?id=eff12");
  });

  test("a link to an extension's kind opens the page its ref_kinds names", () => {
    setResources([
      { kind: "acme_pr", extension: "acme", label: "PR", idPattern: ".*", wikilinks: [], resolve: "", page: "oxplow:oxplow/page?page=acme/pr&ref=%7Bref%7D", icon: "" },
    ]);
    try {
      const entry = edgeToOutboundEntry(edge({ target_kind: "acme_pr", target_id: "12", source_label: "PR 12" }));
      expect(entry.open).toBe("acme_pr:12");
      expect(entry.dead).toBe(false);
    } finally {
      setResources([coreKind("effort", "{ref}"), coreKind("work_item", "{ref}"), coreKind("collector", null)]);
    }
  });

  test("a link to an uninstalled extension's kind is dead, with its raw text, and does not throw", () => {
    const entry = edgeToOutboundEntry(
      edge({ target_kind: "acme_pr", target_id: "12", resolved: false, source_label: null }),
    );
    expect(entry.open).toBeNull();
    expect(entry.dead).toBe(true);
    expect(entry.label).toBe("acme_pr:12");
    expect(dedupeEntriesByTarget([entry, entry])).toHaveLength(1);
  });

  test("an inbound edge reads the source side", () => {
    const entry = edgeToInboundEntry(edge({ source_label: "Fix it" }));
    expect(entry.open).toBe("oxplow:oxplow/work_item?id=tsk1&provider=oxplow");
    expect(entry.label).toBe("Fix it");
  });
});
