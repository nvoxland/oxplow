import { describe, expect, test } from "bun:test";
import {
  agentRef,
  dashboardRef,
  diffRef,
  fileRef,
  findingRef,
  hookEventsRef,
  indexRef,
  newWorkItemRef,
  noteRef,
  workItemRef,
} from "./pageRefs.js";

describe("pageRefs", () => {
  test("agentRef is stable across calls", () => {
    expect(agentRef().id).toBe("agent");
    expect(agentRef().kind).toBe("agent");
  });

  test("fileRef encodes the path", () => {
    expect(fileRef("src/a.ts")).toEqual({
      id: "file:src/a.ts",
      kind: "file",
      payload: { path: "src/a.ts" },
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

  test("noteRef and workItemRef encode their identifiers", () => {
    expect(noteRef("how-x-works").id).toBe("note:how-x-works");
    expect(workItemRef("wi-123").id).toBe("wi:wi-123");
  });

  test("findingRef encodes the finding id", () => {
    expect(findingRef("f-7").id).toBe("finding:f-7");
  });

  test("indexRef returns the same id and kind", () => {
    const ref = indexRef("all-work");
    expect(ref.id).toBe("all-work");
    expect(ref.kind).toBe("all-work");
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

  test("newWorkItemRef has stable create id but item-scoped edit id", () => {
    expect(newWorkItemRef().id).toBe("new-work-item");
    expect(newWorkItemRef({ editingItemId: "wi-42" }).id).toBe("new-work-item:edit:wi-42");
    // Two edit refs for different items get different ids so multiple edit
    // tabs can coexist.
    expect(newWorkItemRef({ editingItemId: "wi-1" }).id).not.toBe(
      newWorkItemRef({ editingItemId: "wi-2" }).id,
    );
  });
});
