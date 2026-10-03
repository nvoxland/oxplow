import { describe, expect, test } from "bun:test";
import {
  decodeTaskDragPayload,
  decodeTaskDragRefs,
  resolveTaskContextRefs,
} from "./agent-context-dnd.js";

// The drift guard that used to live here (asserting this module's copy of
// `application/x-oxplow-task` still equalled ThreadRail's) is gone with the
// duplication itself — both sides now read one constant from
// `dragMimes.ts`, whose own test pins the literal (tsk271).

describe("decodeTaskDragPayload", () => {
  test("returns [] for null/undefined/empty", () => {
    expect(decodeTaskDragPayload(null)).toEqual([]);
    expect(decodeTaskDragPayload(undefined)).toEqual([]);
    expect(decodeTaskDragPayload("")).toEqual([]);
  });

  test("returns [] for malformed JSON", () => {
    expect(decodeTaskDragPayload("not json")).toEqual([]);
    expect(decodeTaskDragPayload("[1,2,3]")).toEqual([]);
  });

  test("returns ids from the itemIds array form", () => {
    const raw = JSON.stringify({ itemIds: ["tsk101", "tsk102", "tsk9301"], fromThreadId: "t-1" });
    expect(decodeTaskDragPayload(raw)).toEqual(["tsk101", "tsk102", "tsk9301"]);
  });

  test("a payload without itemIds carries no tasks", () => {
    const raw = JSON.stringify({ itemId: "tsk101", fromThreadId: "t-1" });
    expect(decodeTaskDragPayload(raw)).toEqual([]);
  });

  test("skips non-string entries in itemIds", () => {
    const raw = JSON.stringify({ itemIds: ["tsk101", 5, null, "tsk102"] });
    expect(decodeTaskDragPayload(raw)).toEqual(["tsk101", "tsk102"]);
  });

  test("returns [] when itemIds is empty and no fallback id", () => {
    const raw = JSON.stringify({ itemIds: [], fromThreadId: "t-1" });
    expect(decodeTaskDragPayload(raw)).toEqual([]);
  });
});

describe("resolveTaskContextRefs", () => {
  test("maps each id through the lookup into a tasks ContextRef", () => {
    const lookup = (id: string) => {
      if (id === "tsk101") return { title: "Alpha", status: "ready" };
      if (id === "tsk102") return { title: "Beta", status: "in_progress" };
      return null;
    };
    const refs = resolveTaskContextRefs(["tsk101", "tsk102"], lookup);
    expect(refs).toEqual([
      { kind: "task", itemId: "tsk101", title: "Alpha", status: "ready" },
      { kind: "task", itemId: "tsk102", title: "Beta", status: "in_progress" },
    ]);
  });

  test("skips ids the lookup doesn't resolve", () => {
    const lookup = (id: string) =>
      id === "tsk101" ? { title: "Alpha", status: "ready" } : null;
    const refs = resolveTaskContextRefs(["tsk999", "tsk101"], lookup);
    expect(refs).toEqual([
      { kind: "task", itemId: "tsk101", title: "Alpha", status: "ready" },
    ]);
  });

  test("returns [] for empty id list", () => {
    expect(resolveTaskContextRefs([], () => null)).toEqual([]);
  });
});

describe("decodeTaskDragRefs", () => {
  test("returns [] when items slice is absent", () => {
    const raw = JSON.stringify({ itemIds: ["tsk101"] });
    expect(decodeTaskDragRefs(raw)).toEqual([]);
  });

  test("returns ContextRefs from the items slice", () => {
    const raw = JSON.stringify({
      itemIds: ["tsk101", "tsk102"],
      items: [
        { id: "tsk101", title: "Alpha", status: "ready" },
        { id: "tsk102", title: "Beta", status: "in_progress" },
      ],
    });
    expect(decodeTaskDragRefs(raw)).toEqual([
      { kind: "task", itemId: "tsk101", title: "Alpha", status: "ready" },
      { kind: "task", itemId: "tsk102", title: "Beta", status: "in_progress" },
    ]);
  });

  test("skips malformed entries but keeps valid ones", () => {
    const raw = JSON.stringify({
      items: [
        { id: "tsk101", title: "Alpha", status: "ready" },
        { id: "", title: "x", status: "y" },
        { id: "tsk9301", title: "Charlie", status: "done" },
        { title: "no id", status: "x" },
      ],
    });
    expect(decodeTaskDragRefs(raw)).toEqual([
      { kind: "task", itemId: "tsk101", title: "Alpha", status: "ready" },
      { kind: "task", itemId: "tsk9301", title: "Charlie", status: "done" },
    ]);
  });

  test("returns [] for malformed JSON", () => {
    expect(decodeTaskDragRefs("not json")).toEqual([]);
    expect(decodeTaskDragRefs(null)).toEqual([]);
  });
});
