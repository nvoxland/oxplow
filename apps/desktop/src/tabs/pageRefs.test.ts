import { describe, expect, test } from "bun:test";
import {
  AGENT_TAB_ID,
  agentRef,
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
  opErrorRef,
  refFromTabId,
  snapshotRef,
  streamSettingsRef,
  threadSettingsRef,
  wikiFreshnessRef,
  wikiPageRef,
  taskRef,
} from "./pageRefs.js";
import { parseRef } from "../refs/ref.js";

describe("pageRefs", () => {
  test("agentRef is the `page:agent` route", () => {
    expect(agentRef().id).toBe("page:agent");
    expect(agentRef().kind).toBe("agent");
    expect(AGENT_TAB_ID).toBe("page:agent");
  });

  test("fileRef encodes the path with a default disk version", () => {
    expect(fileRef("src/a.ts")).toEqual({
      id: "file:src/a.ts",
      kind: "file",
      payload: { path: "src/a.ts", version: { kind: "disk" } },
    });
  });

  test("diffRef is keyed by path, both versions and the label", () => {
    const spec = { path: "src/a.ts", leftVersion: { kind: "ref" as const, ref: "abc" }, rightVersion: { kind: "disk" as const }, baseLabel: "abc" };
    const a = diffRef(spec);
    expect(a.id).toBe("page:diff?path=src/a.ts&left=ref:abc&right=disk");
    expect(a.kind).toBe("diff");
    expect(a.payload).toEqual({ path: "src/a.ts", leftVersion: { kind: "ref", ref: "abc" }, rightVersion: { kind: "disk" }, labelOverride: null });
    // revealLine does not change the id: re-clicking reuses the tab.
    expect(diffRef({ ...spec, revealLine: 7 }).id).toBe(a.id);
    expect(diffRef({ ...spec, rightVersion: { kind: "ref", ref: "xyz" } }).id).not.toBe(a.id);
    expect(diffRef({ ...spec, labelOverride: "wi 3" }).id).toBe("page:diff?path=src/a.ts&left=ref:abc&right=disk&label=wi+3");
  });

  test("wikiPageRef and taskRef encode canonical refs", () => {
    expect(wikiPageRef("how-x-works").id).toBe("wiki:how-x-works");
    // A task is a work item under the oxplow provider (.context/refs.md).
    expect(taskRef("tsk123")).toEqual({
      id: "work_item:oxplow:tsk123",
      kind: "work_item",
      payload: { itemId: "tsk123" },
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
    expect(opErrorRef("oe-1").id).toBe("page:op-error?id=oe-1");
  });

  test("route params stay readable: only the ref-reserved and query-syntax characters are escaped", () => {
    // `/` and `:` are legal raw in a ref id and in a query value; `#`, `=`,
    // `&` and `%` are not.
    expect(externalUrlRef("https://x.test/p?a=1&b=2#frag").id)
      .toBe("page:external-url?url=https://x.test/p?a%3D1%26b%3D2%23frag");
    // Every route id is a valid canonical ref of kind `page`.
    expect(parseRef(externalUrlRef("https://x.test/p?a=1#frag").id)?.kind).toBe("page");
    expect(parseRef(diffRef({ path: "a@b/c%d.ts", leftVersion: { kind: "disk" }, rightVersion: { kind: "disk" }, baseLabel: "" }).id)?.kind).toBe("page");
  });

  test("newTaskRef has stable create id", () => {
    expect(newTaskRef().id).toBe("page:new-task");
    expect(newTaskRef({ parentId: 1 }).id).toBe("page:new-task");
  });

  test("effortDiffRef encodes the effort id under the diff-view route", () => {
    const ref = effortDiffRef("eff42");
    expect(ref.id).toBe("page:diff-view?effort=eff42");
    expect(ref.kind).toBe("diff-view");
    expect(ref.payload).toEqual({ mode: "effort", effortId: "eff42" });
    expect(snapshotRef(112).id).toBe("page:diff-view?snapshot=112");
  });

  test("endpointDiffRef encodes both endpoints; ids are stable + distinct", () => {
    const a = endpointDiffRef(
      { kind: "snapshot", snapshot_id: 1 },
      { kind: "snapshot", snapshot_id: 9 },
    );
    const b = endpointDiffRef(
      { kind: "snapshot", snapshot_id: 1 },
      { kind: "snapshot", snapshot_id: 9 },
    );
    expect(a.id).toBe("page:diff-view?start=s1&end=s9");
    expect(a.id).toBe(b.id);
    expect(a.kind).toBe("diff-view");
    const c = endpointDiffRef(null, { kind: "commit", sha: "abc123" });
    expect(c.id).toBe("page:diff-view?start=none&end=cabc123");
    expect(c.id).not.toBe(a.id);
  });
});

describe("refFromTabId — diff-view", () => {
  test("round-trips an effort diff", () => {
    expect(refFromTabId("page:diff-view?effort=eff42")).toEqual(effortDiffRef("eff42"));
  });

  test("round-trips snapshot↔snapshot endpoints", () => {
    const ref = endpointDiffRef(
      { kind: "snapshot", snapshot_id: 1 },
      { kind: "snapshot", snapshot_id: 9 },
    );
    expect(refFromTabId(ref.id)).toEqual(ref);
  });

  test("round-trips a null-start commit endpoint and a working endpoint", () => {
    const commitRef = endpointDiffRef(null, { kind: "commit", sha: "abc123" });
    expect(refFromTabId(commitRef.id)).toEqual(commitRef);
    const workingRef = endpointDiffRef(
      { kind: "snapshot", snapshot_id: 5 },
      { kind: "working" },
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
    expect((versioned.payload as { version: unknown }).version).toEqual({ kind: "ref", ref: "abc" });
  });

  test("rebuilds payload-bearing kinds from their id", () => {
    expect(refFromTabId("wiki:some-slug")).toEqual(wikiPageRef("some-slug"));
    // `:` inside an id is legal; a naive split on the first colon breaks here.
    expect(refFromTabId("work_item:oxplow:tsk42")).toEqual(taskRef("tsk42"));
    expect(refFromTabId("commit:abc1234")).toEqual(gitCommitRef("abc1234"));
    expect(refFromTabId("metric:oxplow.todos")).toEqual(metricRef("oxplow.todos"));
    // Single snapshot is a diff-view ref now (kind "snapshot" is gone).
    expect(refFromTabId(snapshotRef(112).id)).toEqual(snapshotRef(112));
    expect(refFromTabId("page:external-url?url=https://x.test/p")).toEqual(externalUrlRef("https://x.test/p"));
  });

  test("index routes carry no payload", () => {
    expect(refFromTabId("page:tasks")).toEqual({ id: "page:tasks", kind: "tasks", payload: null });
    expect(refFromTabId("page:git-dashboard")).toEqual({ id: "page:git-dashboard", kind: "git-dashboard", payload: null });
    expect(refFromTabId("page:agent")).toEqual(agentRef());
  });

  test("text that is not a ref, an unknown kind, or an unknown route is null (a dead row, not a broken tab)", () => {
    expect(refFromTabId("tasks")).toBeNull();
    expect(refFromTabId("nope:1")).toBeNull();
    expect(refFromTabId("page:no-such-page")).toBeNull();
    expect(refFromTabId("page:diff-view?bogus=1")).toBeNull();
  });

  test("a diff route rebuilds the payload handleOpenDiff registers", () => {
    const ref = diffRef({ path: "src/a.ts", leftVersion: { kind: "snapshot", id: "3" }, rightVersion: { kind: "ref", ref: "HEAD" }, baseLabel: "x", labelOverride: "eff9" });
    expect(refFromTabId(ref.id)).toEqual(ref);
  });

  test("diskFilePath names the working-tree file a tab id shows, or null", () => {
    expect(diskFilePath("file:src/a.ts")).toBe("src/a.ts");
    expect(diskFilePath("file:src/a%40b.ts")).toBe("src/a@b.ts");
    // A pinned revision is a read-only viewer, not the editor's file.
    expect(diskFilePath("file:src/a.ts@git:HEAD")).toBeNull();
    expect(diskFilePath("page:agent")).toBeNull();
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
      agentRef(),
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
      taskRef("tsk42"),
      wikiPageRef("some-slug"),
      customDashboardRef("dsh1"),
      effortDiffRef("eff9"),
      lensRef("acme/blocked", { stream_id: 2 }),
      lensRef("acme/x", { path: "src/a@b.ts", q: "a=b&c" }),
      fileRef("src/a@b.ts"),
      diffRef({ path: "src/a.ts", leftVersion: { kind: "disk" }, rightVersion: { kind: "ref", ref: "HEAD" }, baseLabel: "HEAD" }),
      opErrorRef("oe-1"),
      streamSettingsRef("str1"),
      threadSettingsRef("thr1"),
      wikiFreshnessRef("data-model"),
      uncommittedChangesRef(),
      fileRef("src/a.ts", { kind: "ref", ref: "HEAD" }),
      externalUrlRef("https://x.test/p?a=1#frag"),
      duplicateBlockRef({
        leftPath: "a.rs", leftStart: 1, leftEnd: 5, leftVersion: { kind: "disk" },
        rightPath: "b.rs", rightStart: 9, rightEnd: 13, rightVersion: { kind: "ref", ref: "abc" },
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
