import { describe, expect, test } from "bun:test";

import { canonicalIdForTarget, dedupeEntriesByTarget, humanRefTypeForTest } from "./useBacklinks.js";
import type { BacklinkEntry } from "./backlinkTypes.js";
import {
  directoryRef,
  fileRef,
  findingRef,
  gitCommitRef,
  taskRef,
  wikiPageRef,
} from "./pageRefs.js";

describe("canonicalIdForTarget", () => {
  test("taskRef returns the integer id as a string", () => {
    // Regression: taskRef stores itemId as a number, but Tauri
    // commands take String — without the stringify, IPC throws and
    // the Outbound dropdown on a task page silently renders empty.
    const ref = taskRef("42");
    const id = canonicalIdForTarget(ref);
    expect(id).toBe("42");
    expect(typeof id).toBe("string");
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

  test("findingRef returns the finding id", () => {
    expect(canonicalIdForTarget(findingRef("fnd-1"))).toBe("fnd-1");
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
      "task_body_mention",
      "summary_task_mention",
      "finding_mention",
      "summary_finding_mention",
      "commit_mention",
      "summary_commit_mention",
    ]) {
      expect(humanRefTypeForTest(rt, null)).toBe("mention");
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
    expect(humanRefTypeForTest("task_link:blocks", null)).toBe("blocks");
    expect(humanRefTypeForTest("task_link:relates_to", null)).toBe("relates to");
    expect(humanRefTypeForTest("task_link:discovered_from", null)).toBe("discovered from");
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
      { ref: fileRef(".oxplow/wiki/local-snapshots.md"), label: ".oxplow/wiki/local-snapshots.md", subtitle: "modified" },
      { ref: wikiPageRef("local-snapshots"), label: "Local Snapshots", subtitle: "created" },
      { ref: wikiPageRef("local-snapshots"), label: "Local Snapshots", subtitle: "mention" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(1);
    expect(out[0].ref.kind).toBe("wiki");
    expect(out[0].label).toBe("Local Snapshots");
    // Subtitles merged in first-seen order, deduped.
    expect(out[0].subtitle).toBe("modified · created · mention");
  });

  test("preserves rows that point at different targets", () => {
    const entries: BacklinkEntry[] = [
      { ref: wikiPageRef("a"), label: "A", subtitle: "mention" },
      { ref: wikiPageRef("b"), label: "B", subtitle: "mention" },
      { ref: fileRef("src/x.ts"), label: "src/x.ts", subtitle: "modified" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(3);
  });

  test("collapses two ref_types pointing at the same wiki page", () => {
    const entries: BacklinkEntry[] = [
      { ref: wikiPageRef("url-schemes"), label: "URL Schemes", subtitle: "created" },
      { ref: wikiPageRef("url-schemes"), label: "URL Schemes", subtitle: "mention" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(1);
    expect(out[0].subtitle).toBe("created · mention");
  });

  test("file path that doesn't look like a wiki shadow stays a file row", () => {
    const entries: BacklinkEntry[] = [
      { ref: fileRef("src/foo.rs"), label: "src/foo.rs", subtitle: "modified" },
      { ref: fileRef("src/foo.rs"), label: "src/foo.rs", subtitle: "mention" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out).toHaveLength(1);
    expect(out[0].ref.kind).toBe("file");
    expect(out[0].subtitle).toBe("modified · mention");
  });

  test("drops blank subtitles cleanly", () => {
    const entries: BacklinkEntry[] = [
      { ref: wikiPageRef("a"), label: "A", subtitle: "" },
      { ref: wikiPageRef("a"), label: "A", subtitle: "mention" },
    ];
    const out = dedupeEntriesByTarget(entries);
    expect(out[0].subtitle).toBe("mention");
  });
});
