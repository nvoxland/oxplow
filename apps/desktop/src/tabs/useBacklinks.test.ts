import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import { dedupeEntriesByTarget, edgeToInboundEntry, edgeToOutboundEntry, graphUriOf, humanRefTypeForTest } from "./useBacklinks.js";
import type { BacklinkEntry } from "./backlinkTypes.js";
import type { BacklinkEdge } from "../api.js";
import { setResources, type Resource } from "../resourceStore.js";
import { collectorUri, commitUri, dirUri, fileUri, pageUri, resourceUri, knowledgeUri, taskUri } from "../uri/uri.js";
import {
  directoryRef,
  fileRef,
  gitCommitRef,
  knowledgePageRef,
  pageTabRef,
  taskTabRef,
} from "./pageRefs.js";

describe("graphUriOf", () => {
  // page_ref rows hang on a resource's URI (.context/resources.md), whichever list.
  test("a task is its own URI", () => {
    const item = taskUri("fake", "W-1");
    expect(graphUriOf(taskTabRef(item))).toBe(item);
  });

  test("a knowledge page, a file and a directory are theirs, a file without its revision", () => {
    expect(graphUriOf(knowledgePageRef("oxwiki", "url-schemes"))).toBe(knowledgeUri("oxwiki", "url-schemes"));
    expect(graphUriOf(fileRef("src/app.ts"))).toBe(fileUri("src/app.ts"));
    expect(graphUriOf(directoryRef("src/components"))).toBe(dirUri("src/components"));
  });

  test("gitCommitRef is the commit's", () => {
    expect(graphUriOf(gitCommitRef("abc1234"))).toBe(commitUri("abc1234"));
  });

  test("any other kind with a page is its own URI, without its view", () => {
  });

  test("a shell route has none", () => {
    expect(graphUriOf(pageTabRef("settings"))).toBeNull();
  });
});

describe("humanRefType", () => {
  test("body-mention ref_types all collapse to 'mention'", () => {
    for (const rt of [
      "wikilink",
      "file_pin",
      "dir_pin",
      "file_mention",
      "dir_mention",
      "task_mention",
      "finding_mention",
      "commit_mention",
    ]) {
      expect(humanRefTypeForTest(rt, null)).toBe("mention");
    }
  });

  // A task's comments' mentions are the item's own edges: the
  // backlink says it's in a comment.
  test("comment-mention ref_types read as 'comment'", () => {
    for (const rt of [
      "comment_file_ref",
      "comment_dir_ref",
      "comment_wikilink",
      "comment_task_mention",
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

  test("typed task links humanize the sub-type", () => {
    expect(humanRefTypeForTest("task_record_link:blocks", null)).toBe("blocks");
    expect(humanRefTypeForTest("task_record_link:relates_to", null)).toBe("relates to");
    expect(humanRefTypeForTest("task_record_link:discovered_from", null)).toBe("discovered from");
  });

  test("finding_path stays as 'found in'", () => {
    expect(humanRefTypeForTest("finding_path", null)).toBe("found in");
  });

  test("unknown ref_types fall through to the raw string", () => {
    expect(humanRefTypeForTest("future_thing", null)).toBe("future_thing");
  });
});

describe("dedupeEntriesByTarget", () => {
  test("collapses an oxwiki page and its on-disk file into one row", () => {
    const entries: BacklinkEntry[] = [
      { open: "oxplow:oxplow/file?path=.oxplow/oxwiki/local-snapshots.md", target: "oxplow:oxplow/file?path=.oxplow/oxwiki/local-snapshots.md", label: ".oxplow/oxwiki/local-snapshots.md", subtitle: "modified" },
      { open: "oxplow:oxplow/knowledge?id=local-snapshots&store=oxwiki", target: "oxplow:oxplow/knowledge?id=local-snapshots&store=oxwiki", label: "Local Snapshots", subtitle: "created" },
      { open: "oxplow:oxplow/knowledge?id=local-snapshots&store=oxwiki", target: "oxplow:oxplow/knowledge?id=local-snapshots&store=oxwiki", label: "Local Snapshots", subtitle: "mention" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(1);
    expect(out[0].open).toBe("oxplow:oxplow/knowledge?id=local-snapshots&store=oxwiki");
    expect(out[0].label).toBe("Local Snapshots");
    // Subtitles merged in first-seen order, deduped.
    expect(out[0].subtitle).toBe("modified · created · mention");
  });

  test("preserves rows that point at different targets", () => {
    const entries: BacklinkEntry[] = [
      { open: "oxplow:oxplow/knowledge?id=a&store=oxwiki", target: "oxplow:oxplow/knowledge?id=a&store=oxwiki", label: "A", subtitle: "mention" },
      { open: "oxplow:oxplow/knowledge?id=b&store=oxwiki", target: "oxplow:oxplow/knowledge?id=b&store=oxwiki", label: "B", subtitle: "mention" },
      { open: "oxplow:oxplow/file?path=src/x.ts", target: "oxplow:oxplow/file?path=src/x.ts", label: "src/x.ts", subtitle: "modified" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(3);
  });

  test("collapses two ref_types pointing at the same knowledge page", () => {
    const entries: BacklinkEntry[] = [
      { open: "oxplow:oxplow/knowledge?id=url-schemes&store=oxwiki", target: "oxplow:oxplow/knowledge?id=url-schemes&store=oxwiki", label: "URL Schemes", subtitle: "created" },
      { open: "oxplow:oxplow/knowledge?id=url-schemes&store=oxwiki", target: "oxplow:oxplow/knowledge?id=url-schemes&store=oxwiki", label: "URL Schemes", subtitle: "mention" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(1);
    expect(out[0].subtitle).toBe("created · mention");
  });

  test("file path that doesn't look like a knowledge page stays a file row", () => {
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
      { open: "oxplow:oxplow/knowledge?id=a&store=oxwiki", target: "oxplow:oxplow/knowledge?id=a&store=oxwiki", label: "A", subtitle: "" },
      { open: "oxplow:oxplow/knowledge?id=a&store=oxwiki", target: "oxplow:oxplow/knowledge?id=a&store=oxwiki", label: "A", subtitle: "mention" },
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
  const CORE = () => [coreKind("request", null), coreKind("task", null), coreKind("collector", null)];
  beforeEach(() => setResources(CORE()));
  afterEach(() => setResources([]));
  const edge = (over: Partial<BacklinkEdge>): BacklinkEdge => ({
    source: taskUri("oxtask", "tsk1"),
    target: knowledgeUri("oxwiki", "x"),
    ref_type: "task_record_link:about",
    source_extra: null,
    source_label: null,
    resolved: true,
    ...over,
  });

  test("an about link to a collector resolves and opens as itself", () => {
    const entry = edgeToOutboundEntry(
      edge({ target: collectorUri("triage", "todos"), source_label: "Triage todos" }),
    );
    expect(entry.dead).toBe(false);
    expect(entry.open).toBe(collectorUri("triage", "todos"));
    expect(entry.label).toBe("Triage todos");
    expect(entry.subtitle).toBe("about");
    expect(entry.linkType).toBe("about");
    expect(entry.target).toBe(collectorUri("triage", "todos"));
  });

  test("a link to a core kind with a page opens it", () => {
    const entry = edgeToOutboundEntry(edge({ target: resourceUri("oxplow/request", { id: "req12" }), source_label: "A request" }));
    expect(entry.open).toBe(resourceUri("oxplow/request", { id: "req12" }));
  });

  test("a link to an extension's resource opens as itself, drawn by its renderer", () => {
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

  test("a link to text that is no URI is dead, with its raw text, and does not throw", () => {
    // Intent moved: an uninstalled extension's URI is a tab now (the fallback
    // page shows it as a dead link), so the dead row is the non-URI one.
    const entry = edgeToOutboundEntry(edge({ target: "gone_kind:12", resolved: false, source_label: null }));
    expect(entry.open).toBeNull();
    expect(entry.dead).toBe(true);
    expect(entry.label).toBe("gone_kind:12");
    expect(dedupeEntriesByTarget([entry, entry])).toHaveLength(1);
  });

  test("an inbound edge reads the source side", () => {
    const entry = edgeToInboundEntry(edge({ source_label: "Fix it" }));
    expect(entry.open).toBe("oxplow:oxplow/task?id=tsk1&provider=oxtask");
    expect(entry.label).toBe("Fix it");
  });
});
