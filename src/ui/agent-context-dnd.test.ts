import { describe, expect, test } from "bun:test";
import {
  WORK_ITEM_DRAG_MIME_VALUE,
  decodeWorkItemDragPayload,
  decodeWorkItemDragRefs,
  resolveWorkItemContextRefs,
} from "./agent-context-dnd.js";
import { WORK_ITEM_DRAG_MIME } from "./components/ThreadRail.js";

describe("WORK_ITEM_DRAG_MIME_VALUE", () => {
  test("matches the canonical MIME string from ThreadRail", () => {
    // Guards against drift between the constant agent-context-dnd holds
    // (so it can decode payloads without importing the React tree) and
    // the one ThreadRail/WorkGroupList encode with.
    expect(WORK_ITEM_DRAG_MIME_VALUE).toBe(WORK_ITEM_DRAG_MIME);
  });
});

describe("decodeWorkItemDragPayload", () => {
  test("returns [] for null/undefined/empty", () => {
    expect(decodeWorkItemDragPayload(null)).toEqual([]);
    expect(decodeWorkItemDragPayload(undefined)).toEqual([]);
    expect(decodeWorkItemDragPayload("")).toEqual([]);
  });

  test("returns [] for malformed JSON", () => {
    expect(decodeWorkItemDragPayload("not json")).toEqual([]);
    expect(decodeWorkItemDragPayload("[1,2,3]")).toEqual([]);
  });

  test("returns ids from the itemIds array form", () => {
    const raw = JSON.stringify({ itemIds: ["wi-a", "wi-b", "wi-c"], fromThreadId: "t-1" });
    expect(decodeWorkItemDragPayload(raw)).toEqual(["wi-a", "wi-b", "wi-c"]);
  });

  test("falls back to single itemId when itemIds is absent", () => {
    const raw = JSON.stringify({ itemId: "wi-a", fromThreadId: "t-1" });
    expect(decodeWorkItemDragPayload(raw)).toEqual(["wi-a"]);
  });

  test("prefers itemIds when both are present", () => {
    const raw = JSON.stringify({ itemId: "wi-a", itemIds: ["wi-b", "wi-c"] });
    expect(decodeWorkItemDragPayload(raw)).toEqual(["wi-b", "wi-c"]);
  });

  test("skips non-string entries in itemIds", () => {
    const raw = JSON.stringify({ itemIds: ["wi-a", 42, null, "wi-b"] });
    expect(decodeWorkItemDragPayload(raw)).toEqual(["wi-a", "wi-b"]);
  });

  test("returns [] when itemIds is empty and no fallback id", () => {
    const raw = JSON.stringify({ itemIds: [], fromThreadId: "t-1" });
    expect(decodeWorkItemDragPayload(raw)).toEqual([]);
  });
});

describe("resolveWorkItemContextRefs", () => {
  test("maps each id through the lookup into a work-item ContextRef", () => {
    const lookup = (id: string) => {
      if (id === "wi-a") return { title: "Alpha", status: "ready" };
      if (id === "wi-b") return { title: "Beta", status: "in_progress" };
      return null;
    };
    const refs = resolveWorkItemContextRefs(["wi-a", "wi-b"], lookup);
    expect(refs).toEqual([
      { kind: "work-item", itemId: "wi-a", title: "Alpha", status: "ready" },
      { kind: "work-item", itemId: "wi-b", title: "Beta", status: "in_progress" },
    ]);
  });

  test("skips ids the lookup doesn't resolve", () => {
    const lookup = (id: string) =>
      id === "wi-a" ? { title: "Alpha", status: "ready" } : null;
    const refs = resolveWorkItemContextRefs(["wi-missing", "wi-a"], lookup);
    expect(refs).toEqual([
      { kind: "work-item", itemId: "wi-a", title: "Alpha", status: "ready" },
    ]);
  });

  test("returns [] for empty id list", () => {
    expect(resolveWorkItemContextRefs([], () => null)).toEqual([]);
  });
});

describe("decodeWorkItemDragRefs", () => {
  test("returns [] when items slice is absent", () => {
    const raw = JSON.stringify({ itemIds: ["wi-a"] });
    expect(decodeWorkItemDragRefs(raw)).toEqual([]);
  });

  test("returns ContextRefs from the items slice", () => {
    const raw = JSON.stringify({
      itemIds: ["wi-a", "wi-b"],
      items: [
        { id: "wi-a", title: "Alpha", status: "ready" },
        { id: "wi-b", title: "Beta", status: "in_progress" },
      ],
    });
    expect(decodeWorkItemDragRefs(raw)).toEqual([
      { kind: "work-item", itemId: "wi-a", title: "Alpha", status: "ready" },
      { kind: "work-item", itemId: "wi-b", title: "Beta", status: "in_progress" },
    ]);
  });

  test("skips malformed entries but keeps valid ones", () => {
    const raw = JSON.stringify({
      items: [
        { id: "wi-a", title: "Alpha", status: "ready" },
        { id: 7, title: "x", status: "y" },
        { id: "wi-c", title: "Charlie", status: "done" },
        { title: "no id", status: "x" },
      ],
    });
    expect(decodeWorkItemDragRefs(raw)).toEqual([
      { kind: "work-item", itemId: "wi-a", title: "Alpha", status: "ready" },
      { kind: "work-item", itemId: "wi-c", title: "Charlie", status: "done" },
    ]);
  });

  test("returns [] for malformed JSON", () => {
    expect(decodeWorkItemDragRefs("not json")).toEqual([]);
    expect(decodeWorkItemDragRefs(null)).toEqual([]);
  });
});
