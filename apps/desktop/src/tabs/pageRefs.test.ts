import { describe, expect, test } from "bun:test";
import {
  agentRef,
  customDashboardRef,
  dashboardRef,
  directoryRef,
  diffRef,
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
  refFromTabId,
  snapshotRef,
  wikiPageRef,
  taskRef,
} from "./pageRefs.js";

describe("pageRefs", () => {
  test("agentRef is stable across calls", () => {
    expect(agentRef().id).toBe("agent");
    expect(agentRef().kind).toBe("agent");
  });

  test("fileRef encodes the path with a default disk version", () => {
    expect(fileRef("src/a.ts")).toEqual({
      id: "file:src/a.ts",
      kind: "file",
      payload: { path: "src/a.ts", version: { kind: "disk" } },
    });
  });

  test("diffRef produces stable ids for identical payloads", () => {
    const a = diffRef({ path: "src/a.ts", fromRef: "abc", toRef: "def" });
    const b = diffRef({ path: "src/a.ts", fromRef: "abc", toRef: "def" });
    expect(a.id).toBe(b.id);
  });

  test("diffRef ids differ when refs differ", () => {
    const a = diffRef({ path: "src/a.ts", fromRef: "abc", toRef: "def" });
    const b = diffRef({ path: "src/a.ts", fromRef: "abc", toRef: "xyz" });
    expect(a.id).not.toBe(b.id);
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


  test("indexRef returns the same id and kind", () => {
    const ref = indexRef("tasks");
    expect(ref.id).toBe("tasks");
    expect(ref.kind).toBe("tasks");
  });

  test("dashboardRef encodes the variant", () => {
    expect(dashboardRef("visits").id).toBe("dashboard:visits");
  });

  test("hookEventsRef returns the hook-events index ref", () => {
    const ref = hookEventsRef();
    expect(ref.id).toBe("hook-events");
    expect(ref.kind).toBe("hook-events");
  });

  test("newTaskRef has stable create id", () => {
    expect(newTaskRef().id).toBe("new-task");
    expect(newTaskRef({ parentId: 1 }).id).toBe("new-task");
  });

  test("effortDiffRef encodes the effort id under the diff-view kind", () => {
    const ref = effortDiffRef("eff42");
    expect(ref.id).toBe("diff-view:effort:eff42");
    expect(ref.kind).toBe("diff-view");
    expect(ref.payload).toEqual({ mode: "effort", effortId: "eff42" });
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
    expect(a.id).toBe("diff-view:endpoints:s1..s9");
    expect(a.id).toBe(b.id);
    expect(a.kind).toBe("diff-view");
    const c = endpointDiffRef(null, { kind: "commit", sha: "abc123" });
    expect(c.id).toBe("diff-view:endpoints:none..cabc123");
    expect(c.id).not.toBe(a.id);
  });
});

describe("refFromTabId — diff-view", () => {
  test("round-trips an effort diff", () => {
    expect(refFromTabId("diff-view:effort:eff42")).toEqual(effortDiffRef("eff42"));
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
    const r = refFromTabId("file:Cargo.toml");
    expect(r.kind).toBe("file");
    expect((r.payload as { path: string }).path).toBe("Cargo.toml");
    expect(r.id).toBe(fileRef("Cargo.toml").id);
  });

  test("handles nested paths and a versioned-viewer revision", () => {
    expect((refFromTabId("file:src/a/b.ts").payload as { path: string }).path).toBe("src/a/b.ts");
    const versioned = refFromTabId("file:src/x.ts@git:abc");
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
    expect(refFromTabId("external-url:https://x.test/p")).toEqual(externalUrlRef("https://x.test/p"));
  });

  test("uncommitted-changes drops an old drilldown scope suffix", () => {
    expect(refFromTabId("uncommitted-changes:dir:src")).toEqual(uncommittedChangesRef());
  });

  test("index/dashboard ids carry no payload (id is the kind)", () => {
    expect(refFromTabId("tasks")).toEqual({ id: "tasks", kind: "tasks", payload: null });
    expect(refFromTabId("git-dashboard")).toEqual({
      id: "git-dashboard",
      kind: "git-dashboard",
      payload: null,
    });
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
      newTaskRef(),
      snapshotRef(112),
      taskRef("tsk42"),
      wikiPageRef("some-slug"),
      customDashboardRef("dsh1"),
      effortDiffRef("eff9"),
      lensRef("acme/blocked", { stream_id: 2 }),
      fileRef("src/a@b.ts"),
      fileRef("src/a.ts", { kind: "ref", ref: "HEAD" }),
      externalUrlRef("https://x.test/p?a=1#frag"),
      duplicateBlockRef({
        leftPath: "a.rs", leftStart: 1, leftEnd: 5, leftVersion: { kind: "disk" },
        rightPath: "b.rs", rightStart: 9, rightEnd: 13, rightVersion: { kind: "ref", ref: "abc" },
      }),
    ];
    for (const ref of refs) {
      const rebuilt = refFromTabId(ref.id);
      expect(rebuilt.id).toBe(ref.id);
      expect(rebuilt.kind).toBe(ref.kind);
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
    expect(r.payload).toEqual({ lensId: "x/effort-tests", params: { effort_id: 12, label: "a b" } });
    expect(refFromTabId(r.id)).toEqual(r);
    expect(lensRef("x/y", {}).id).toBe("lens:x/y");
  });
});
