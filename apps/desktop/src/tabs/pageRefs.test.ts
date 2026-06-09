import { describe, expect, test } from "bun:test";
import {
  agentRef,
  dashboardRef,
  diffRef,
  externalUrlRef,
  fileRef,
  findingRef,
  gitCommitRef,
  hookEventsRef,
  indexRef,
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
    expect(refFromTabId("snapshot:112")).toEqual(snapshotRef(112));
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
