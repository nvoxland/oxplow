import { describe, expect, test } from "bun:test";
import {
  agentSessionRef,
  customDashboardRef,
  dashboardRef,
  directoryRef,
  diffRef,
  diskFilePath,
  duplicateBlockRef,
  effortDiffRef,
  endpointDiffRef,
  externalUrlRef,
  fileRef,
  gitCommitRef,
  uncommittedChangesRef,
  hookEventsRef,
  indexRef,
  lensRef,
  metricRef,
  newTaskRef,
  newSessionRef,
  alertsRef,
  refFromTabId,
  pageKindOf,
  snapshotRef,
  streamSettingsRef,
  turnRef,
  threadSettingsRef,
  wikiFreshnessRef,
  wikiPageRef,
  workItemTabRef,
} from "./pageRefs.js";
import { parseRef } from "../refs/ref.js";

describe("pageRefs", () => {
  test("an agent session's tab is its ref, pinned; the picker is a closable route", () => {
    expect(agentSessionRef("ses3")).toEqual({
      id: "agent_session:ses3",
      kind: "agent_session",
      payload: { sessionId: "ses3" },
      pinned: true,
    });
    expect(pageKindOf("agent_session:ses3")).toBe("agent_session");
    expect(newSessionRef()).toEqual({ id: "page:new-session", kind: "new-session", payload: null });
    expect(refFromTabId("page:agent")).toBeNull();
  });

  test("fileRef encodes the path with a default disk version", () => {
    expect(fileRef("src/a.ts")).toEqual({
      id: "file:src/a.ts",
      kind: "file",
      payload: { path: "src/a.ts", version: "working" },
    });
  });

  test("diffRef is keyed by path, both versions and the label", () => {
    const spec = { path: "src/a.ts", leftVersion: "git:abc", rightVersion: "working", baseLabel: "abc" };
    const a = diffRef(spec);
    expect(a.id).toBe("page:diff?path=src/a.ts&left=git:abc&right=working");
    expect(a.kind).toBe("diff");
    expect(a.payload).toEqual({ path: "src/a.ts", leftVersion: "git:abc", rightVersion: "working", labelOverride: null });
    // revealLine does not change the id: re-clicking reuses the tab.
    expect(diffRef({ ...spec, revealLine: 7 }).id).toBe(a.id);
    expect(diffRef({ ...spec, rightVersion: "git:xyz" }).id).not.toBe(a.id);
    expect(diffRef({ ...spec, labelOverride: "wi 3" }).id).toBe("page:diff?path=src/a.ts&left=git:abc&right=working&label=wi+3");
  });

  test("wikiPageRef and workItemTabRef encode canonical refs", () => {
    expect(wikiPageRef("how-x-works").id).toBe("wiki:how-x-works");
    // A work item's tab is its ref, whichever list (.context/refs.md).
    expect(workItemTabRef("work_item:oxplow:tsk123")).toEqual({
      id: "work_item:oxplow:tsk123",
      kind: "work_item",
      payload: { ref: "work_item:oxplow:tsk123" },
    });
    expect(gitCommitRef("abc1234")).toEqual({ id: "commit:abc1234", kind: "commit", payload: { sha: "abc1234" } });
    expect(metricRef("oxplow.todos")).toEqual({
      id: "metric:oxplow.todos",
      kind: "metric",
      payload: { metricKey: "oxplow.todos" },
    });
  });


  test("index pages are `page:<kind>` routes", () => {
    const ref = indexRef("tasks");
    expect(ref.id).toBe("page:tasks");
    expect(ref.kind).toBe("tasks");
    expect(hookEventsRef()).toEqual({ id: "page:hook-events", kind: "hook-events", payload: null });
    expect(uncommittedChangesRef().id).toBe("page:uncommitted-changes");
  });

  test("routes with a subject carry it as query params", () => {
    expect(dashboardRef("visits").id).toBe("page:dashboard?variant=visits");
    expect(wikiFreshnessRef("data-model").id).toBe("page:wiki-freshness?slug=data-model");
    expect(customDashboardRef("dsh3").id).toBe("page:custom-dashboard?id=dsh3");
    expect(streamSettingsRef("str1").id).toBe("page:stream-settings?stream=str1");
    expect(threadSettingsRef("thr1").id).toBe("page:thread-settings?thread=thr1");
    expect(alertsRef().id).toBe("page:alerts");
  });

  test("route params stay readable: only the ref-reserved and query-syntax characters are escaped", () => {
    // `/` and `:` are legal raw in a ref id and in a query value; `#`, `=`,
    // `&` and `%` are not.
    expect(externalUrlRef("https://x.test/p?a=1&b=2#frag").id)
      .toBe("page:external-url?url=https://x.test/p?a%3D1%26b%3D2%23frag");
    // Every route id is a valid canonical ref of kind `page`.
    expect(parseRef(externalUrlRef("https://x.test/p?a=1#frag").id)?.kind).toBe("page");
    expect(parseRef(diffRef({ path: "a@b/c%d.ts", leftVersion: "working", rightVersion: "working", baseLabel: "" }).id)?.kind).toBe("page");
  });

  test("newTaskRef has stable create id", () => {
    expect(newTaskRef().id).toBe("page:new-task");
    expect(newTaskRef({ parentId: 1 }).id).toBe("page:new-task");
  });

  test("an effort, snapshot or turn page is its canonical ref (P2.11)", () => {
    const effort = effortDiffRef("eff42");
    expect(effort).toEqual({
      id: "effort:eff42",
      kind: "effort",
      payload: { mode: "effort", effortId: "eff42" },
    });
    expect(snapshotRef(112)).toEqual({
      id: "snapshot:112",
      kind: "snapshot",
      payload: { mode: "snapshot", snapshotId: 112 },
    });
    expect(turnRef("trn7")).toEqual({
      id: "turn:trn7",
      kind: "turn",
      payload: { mode: "turn", turnId: "trn7" },
    });
    for (const ref of [effort, snapshotRef(112), turnRef("trn7")]) {
      expect(refFromTabId(ref.id)).toEqual(ref);
      expect(pageKindOf(ref.id)).toBe(ref.kind);
    }
  });

  test("a file path with ref-reserved characters round-trips (P2.11)", () => {
    const path = "docs/a@b#c.md";
    const ref = fileRef(path);
    expect(ref.id).toBe("file:docs/a%40b%23c.md");
    expect(diskFilePath(ref.id)).toBe(path);
    expect(refFromTabId(ref.id)).toEqual(ref);
  });

  test("endpointDiffRef encodes both endpoints; ids are stable + distinct", () => {
    const a = endpointDiffRef(
      "snap:1",
      "snap:9",
    );
    const b = endpointDiffRef(
      "snap:1",
      "snap:9",
    );
    expect(a.id).toBe("page:diff-view?start=snap:1&end=snap:9");
    expect(a.id).toBe(b.id);
    expect(a.kind).toBe("diff-view");
    const c = endpointDiffRef(null, "git:abc123");
    expect(c.id).toBe("page:diff-view?start=none&end=git:abc123");
    expect(c.id).not.toBe(a.id);
  });
});

describe("refFromTabId — diff-view", () => {
  test("an effort is its own page now, not a diff-view route", () => {
    expect(refFromTabId("page:diff-view?effort=eff42")).toBeNull();
  });

  test("round-trips snapshot↔snapshot endpoints", () => {
    const ref = endpointDiffRef(
      "snap:1",
      "snap:9",
    );
    expect(refFromTabId(ref.id)).toEqual(ref);
  });

  test("round-trips a null-start commit endpoint and a working endpoint", () => {
    const commitRef = endpointDiffRef(null, "git:abc123");
    expect(refFromTabId(commitRef.id)).toEqual(commitRef);
    const workingRef = endpointDiffRef(
      "snap:5",
      "working",
    );
    expect(refFromTabId(workingRef.id)).toEqual(workingRef);
  });
});

describe("refFromTabId", () => {
  test("rebuilds a file ref with its path payload (the rail-History bug)", () => {
    const r = refFromTabId("file:Cargo.toml")!;
    expect(r.kind).toBe("file");
    expect((r.payload as { path: string }).path).toBe("Cargo.toml");
    expect(r.id).toBe(fileRef("Cargo.toml").id);
  });

  test("handles nested paths and a versioned-viewer revision", () => {
    expect((refFromTabId("file:src/a/b.ts")!.payload as { path: string }).path).toBe("src/a/b.ts");
    const versioned = refFromTabId("file:src/x.ts@git:abc")!;
    expect((versioned.payload as { path: string }).path).toBe("src/x.ts");
    expect((versioned.payload as { version: unknown }).version).toEqual("git:abc");
  });

  test("rebuilds payload-bearing kinds from their id", () => {
    expect(refFromTabId("wiki:some-slug")).toEqual(wikiPageRef("some-slug"));
    // `:` inside an id is legal; a naive split on the first colon breaks here.
    expect(refFromTabId("work_item:oxplow:tsk42")).toEqual(workItemTabRef("work_item:oxplow:tsk42"));
    expect(refFromTabId("commit:abc1234")).toEqual(gitCommitRef("abc1234"));
    expect(refFromTabId("metric:oxplow.todos")).toEqual(metricRef("oxplow.todos"));
    // Single snapshot is a diff-view ref now (kind "snapshot" is gone).
    expect(refFromTabId(snapshotRef(112).id)).toEqual(snapshotRef(112));
    expect(refFromTabId("page:external-url?url=https://x.test/p")).toEqual(externalUrlRef("https://x.test/p"));
  });

  test("index routes carry no payload", () => {
    expect(refFromTabId("page:tasks")).toEqual({ id: "page:tasks", kind: "tasks", payload: null });
    expect(refFromTabId("page:git-dashboard")).toEqual({ id: "page:git-dashboard", kind: "git-dashboard", payload: null });
  });

  test("text that is not a ref, an unknown kind, or an unknown route is null (a dead row, not a broken tab)", () => {
    expect(refFromTabId("tasks")).toBeNull();
    expect(refFromTabId("nope:1")).toBeNull();
    expect(refFromTabId("page:no-such-page")).toBeNull();
    expect(refFromTabId("page:diff-view?bogus=1")).toBeNull();
  });

  test("a diff route rebuilds the payload handleOpenDiff registers", () => {
    const ref = diffRef({ path: "src/a.ts", leftVersion: "snap:3", rightVersion: "git:HEAD", baseLabel: "x", labelOverride: "eff9" });
    expect(refFromTabId(ref.id)).toEqual(ref);
  });

  test("diskFilePath names the working-tree file a tab id shows, or null", () => {
    expect(diskFilePath("file:src/a.ts")).toBe("src/a.ts");
    expect(diskFilePath("file:src/a%40b.ts")).toBe("src/a@b.ts");
    // A pinned revision is a read-only viewer, not the editor's file.
    expect(diskFilePath("file:src/a.ts@git:HEAD")).toBeNull();
    expect(diskFilePath("agent_session:ses3")).toBeNull();
    expect(diskFilePath("wiki:a")).toBeNull();
  });

  // tsk163: a ref that can't be rebuilt from its own id is a dead row — the
  // launcher's Recent, the rail's History, and the Go To page all reopen pages
  // by id. Two constructors had drifted: `dashboard:<variant>` had no case at
  // all (so every variant reopened as Planning), and `dir:` was matched under
  // the spelling "directory" (so the case could never fire and the page never
  // opened). Round-trip EVERY constructor rather than spot-checking, so the
  // next one to drift fails here.
  test("every ref rebuilds from its own tab id", () => {
    const refs = [
      agentSessionRef("ses3"),
      newSessionRef(),
      dashboardRef("visits"),
      directoryRef("src/components"),
      externalUrlRef("https://x.test/p"),
      fileRef("src/a.ts"),
      gitCommitRef("abc123"),
      hookEventsRef(),
      indexRef("tasks"),
      indexRef("git-dashboard"),
      metricRef("oxplow.todos"),
      snapshotRef(112),
      workItemTabRef("work_item:oxplow:tsk42"),
      wikiPageRef("some-slug"),
      customDashboardRef("dsh1"),
      effortDiffRef("eff9"),
      lensRef("acme/blocked", { stream_id: 2 }),
      lensRef("acme/x", { path: "src/a@b.ts", q: "a=b&c" }),
      fileRef("src/a@b.ts"),
      diffRef({ path: "src/a.ts", leftVersion: "working", rightVersion: "git:HEAD", baseLabel: "HEAD" }),
      alertsRef(),
      streamSettingsRef("str1"),
      threadSettingsRef("thr1"),
      wikiFreshnessRef("data-model"),
      uncommittedChangesRef(),
      fileRef("src/a.ts", "git:HEAD"),
      externalUrlRef("https://x.test/p?a=1#frag"),
      duplicateBlockRef({
        leftPath: "a.rs", leftStart: 1, leftEnd: 5, leftVersion: "working",
        rightPath: "b.rs", rightStart: 9, rightEnd: 13, rightVersion: "git:abc",
      }),
    ];
    for (const ref of refs) {
      const rebuilt = refFromTabId(ref.id);
      expect(rebuilt, ref.id).toEqual(ref);
    }
  });

  test("the Go To dashboard reopens as itself", () => {
    expect(refFromTabId(dashboardRef("visits").id)).toEqual(dashboardRef("visits"));
  });

  test("a directory page reopens with its path payload", () => {
    expect(refFromTabId(directoryRef("src/components").id)).toEqual(
      directoryRef("src/components"),
    );
  });

  test("lensRef ids the lens and round-trips through refFromTabId", () => {
    const r = lensRef("review/waiting");
    expect(r).toEqual({ id: "lens:review/waiting", kind: "lens", payload: { lensId: "review/waiting" } });
    expect(refFromTabId(r.id)).toEqual(r);
  });

  test("lensRef carries params in the id, sorted, and round-trips them", () => {
    const r = lensRef("x/effort-tests", { effort_id: 12, label: "a b" });
    expect(r.id).toBe("lens:x/effort-tests?effort_id=12&label=a+b");
    expect(parseRef(r.id)?.kind).toBe("lens");
    expect(r.payload).toEqual({ lensId: "x/effort-tests", params: { effort_id: 12, label: "a b" } });
    expect(refFromTabId(r.id)).toEqual(r);
    expect(lensRef("x/y", {}).id).toBe("lens:x/y");
  });
});

// Every list's work item opens the one work item page, by its ref.
test("a work item ref opens for every provider", () => {
  const theirs = workItemTabRef("work_item:fake:W-1");
  expect(theirs).toEqual({ id: "work_item:fake:W-1", kind: "work_item", payload: { ref: "work_item:fake:W-1" } });
  expect(refFromTabId("work_item:fake:W-1")).toEqual(theirs);
  expect(refFromTabId("work_item:oxplow:tsk3")).toEqual(workItemTabRef("work_item:oxplow:tsk3"));
});

// P9.D3: where a unified-search hit opens. A hit of an extension's ref
// kind (a searchable kind) opens that kind's page with `?ref=`, as a
// `[[pr:12]]` does; core kinds keep their pages; a note has none.
test("searchHitTarget routes core kinds and an extension's kind", async () => {
  const { searchHitTarget, workItemTabRef, wikiPageRef, commentsRef, extPageRef } = await import("./pageRefs.js");
  const { setRefKinds } = await import("../refKinds.js");
  setRefKinds([
    { kind: "acme_pr", extension: "acme", label: "Pull request", idPattern: "^\\d+$", wikilinks: ["pr"], resolve: "v_acme_prs", page: "page:ext.acme.pr", icon: "git-pull-request" },
  ]);
  try {
    // A work item's hit carries its ref after `work_item:`.
    expect(searchHitTarget({ kind: "work_item", ref_id: "oxplow:tsk1" })).toEqual({ page: workItemTabRef("work_item:oxplow:tsk1") });
    expect(searchHitTarget({ kind: "wiki", ref_id: "home" })).toEqual({ page: wikiPageRef("home") });
    expect(searchHitTarget({ kind: "comment", ref_id: "7" })).toEqual({ page: commentsRef() });
    expect(searchHitTarget({ kind: "file", ref_id: "src/a.ts" })).toEqual({ file: "src/a.ts" });
    expect(searchHitTarget({ kind: "note", ref_id: "3" })).toBeNull();
    expect(searchHitTarget({ kind: "acme_pr", ref_id: "12" })).toEqual({
      page: extPageRef("acme", "pr", { ref: "acme_pr:12" }),
    });
    // A kind nobody declares (its extension is gone) opens nothing.
    expect(searchHitTarget({ kind: "gone_kind", ref_id: "1" })).toBeNull();
  } finally {
    setRefKinds([]);
  }
});

