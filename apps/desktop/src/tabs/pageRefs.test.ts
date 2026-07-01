import { describe, expect, test } from "bun:test";
import {
  agentRef,
  dashboardRef,
  diffRef,
  effortDiffRef,
  endpointDiffRef,
  externalUrlRef,
  fileRef,
  findingRef,
  gitCommitRef,
  hookEventsRef,
  indexRef,
  metricRecordingRef,
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

  test("wikiPageRef and taskRef encode their identifiers", () => {
    expect(wikiPageRef("how-x-works").id).toBe("wiki:how-x-works");
    expect(taskRef("123").id).toBe("task:123");
  });

  test("findingRef encodes the finding id", () => {
    expect(findingRef("f-7").id).toBe("finding:f-7");
  });

  test("indexRef returns the same id and kind", () => {
    const ref = indexRef("tasks");
    expect(ref.id).toBe("tasks");
    expect(ref.kind).toBe("tasks");
  });

  test("dashboardRef encodes the variant", () => {
    expect(dashboardRef("planning").id).toBe("dashboard:planning");
    expect(dashboardRef("review").id).toBe("dashboard:review");
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

describe("refFromTabId — metric-recording", () => {
  test("round-trips the capture id + metric key (the restore needs both)", () => {
    // tsk46: the finding read is keyed by (metricKey, captureId), so a tab
    // restored from history must recover the key from the id alone.
    const ref = metricRecordingRef(42, { metricKey: "oxplow.todos" });
    expect(ref.id).toBe("metric-recording:42:oxplow.todos");
    const restored = refFromTabId(ref.id);
    expect(restored?.kind).toBe("metric-recording");
    expect(restored?.payload).toEqual({ captureId: 42, metricKey: "oxplow.todos" });
  });

  test("a key-less legacy id still restores by capture id", () => {
    const restored = refFromTabId("metric-recording:7");
    expect(restored?.payload).toEqual({ captureId: 7 });
  });
});

describe("refFromTabId", () => {
  test("rebuilds a file ref with its path payload (the rail-History bug)", () => {
    const r = refFromTabId("file:Cargo.toml");
    expect(r.kind).toBe("file");
    expect((r.payload as { path: string }).path).toBe("Cargo.toml");
    expect(r.id).toBe(fileRef("Cargo.toml").id);
  });

  test("handles nested paths and strips a versioned-viewer fragment", () => {
    expect((refFromTabId("file:src/a/b.ts").payload as { path: string }).path).toBe("src/a/b.ts");
    expect((refFromTabId("file:src/x.ts:@abc").payload as { path: string }).path).toBe("src/x.ts");
  });

  test("rebuilds payload-bearing kinds from their id", () => {
    expect(refFromTabId("wiki:some-slug")).toEqual(wikiPageRef("some-slug"));
    expect(refFromTabId("task:42")).toEqual(taskRef("42"));
    // Single snapshot is a diff-view ref now (kind "snapshot" is gone).
    expect(refFromTabId(snapshotRef(112).id)).toEqual(snapshotRef(112));
    expect(refFromTabId("external-url:https://x.test/p")).toEqual(externalUrlRef("https://x.test/p"));
  });

  test("git-commit drops a scope suffix to the bare sha", () => {
    expect(refFromTabId("git-commit:abc123:working:src/a.ts")).toEqual(gitCommitRef("abc123"));
  });

  test("index/dashboard ids carry no payload (id is the kind)", () => {
    expect(refFromTabId("tasks")).toEqual({ id: "tasks", kind: "tasks", payload: null });
    expect(refFromTabId("git-dashboard")).toEqual({
      id: "git-dashboard",
      kind: "git-dashboard",
      payload: null,
    });
  });
});
