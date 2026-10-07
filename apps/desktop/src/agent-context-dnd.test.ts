import { describe, expect, test } from "bun:test";
import { decodeContextRef, decodeWorkItemDrag, workItemDragRefs } from "./agent-context-dnd.js";

// A work-item drag carries refs — any list's — and each item's title and
// state, so the agent terminal adds them without a lookup.
describe("decodeWorkItemDrag", () => {
  test("reads the refs, items, source list and epic", () => {
    const raw = JSON.stringify({
      refs: ["work_item:oxplow:tsk1", "work_item:issues:ENG-2"],
      items: [
        { ref: "work_item:oxplow:tsk1", title: "Alpha", state: "todo" },
        { ref: "work_item:issues:ENG-2", title: "Beta", state: "in_progress" },
      ],
      fromThreadId: "thr3",
      parentEpicRef: "work_item:oxplow:tsk9",
    });
    expect(decodeWorkItemDrag(raw)).toEqual({
      refs: ["work_item:oxplow:tsk1", "work_item:issues:ENG-2"],
      items: [
        { ref: "work_item:oxplow:tsk1", title: "Alpha", state: "todo" },
        { ref: "work_item:issues:ENG-2", title: "Beta", state: "in_progress" },
      ],
      fromThreadId: "thr3",
      parentEpicRef: "work_item:oxplow:tsk9",
    });
  });

  test("drops malformed entries and refuses junk", () => {
    const raw = JSON.stringify({ refs: ["a", 3, ""], items: [{ ref: "a", title: "A", state: "todo" }, { ref: "b" }, null] });
    expect(decodeWorkItemDrag(raw)).toEqual({ refs: ["a"], items: [{ ref: "a", title: "A", state: "todo" }], fromThreadId: null });
    expect(decodeWorkItemDrag("not json")).toBeNull();
    expect(decodeWorkItemDrag("[1,2]")).toBeNull();
    expect(decodeWorkItemDrag(null)).toBeNull();
  });

  test("each resolved item becomes a context ref", () => {
    const drag = decodeWorkItemDrag(
      JSON.stringify({ refs: ["work_item:oxplow:tsk1"], items: [{ ref: "work_item:oxplow:tsk1", title: "Alpha", state: "todo" }] }),
    );
    expect(workItemDragRefs(drag)).toEqual([{ kind: "work_item", ref: "work_item:oxplow:tsk1", title: "Alpha", state: "todo" }]);
    expect(workItemDragRefs(null)).toEqual([]);
  });
});

// A lens row drags the ref it links to (any canonical ref), so the
// terminal must read that payload back, not only files, wiki pages and
// work items.
describe("decodeContextRef", () => {
  test("reads every kind a drag source sets", () => {
    expect(decodeContextRef(JSON.stringify({ kind: "file", path: "a.ts" }))).toEqual({ kind: "file", path: "a.ts" });
    expect(decodeContextRef(JSON.stringify({ kind: "wiki", slug: "s" }))).toEqual({ kind: "wiki", slug: "s" });
    expect(
      decodeContextRef(JSON.stringify({ kind: "work_item", ref: "work_item:issues:ENG-1", title: "T", state: "todo" })),
    ).toEqual({ kind: "work_item", ref: "work_item:issues:ENG-1", title: "T", state: "todo" });
    expect(decodeContextRef(JSON.stringify({ kind: "ref", ref: "work_item:oxplow:tsk12" }))).toEqual({
      kind: "ref",
      ref: "work_item:oxplow:tsk12",
    });
  });

  test("refuses a ref that isn't canonical, and junk", () => {
    expect(decodeContextRef(JSON.stringify({ kind: "ref", ref: "not a ref" }))).toBeNull();
    expect(decodeContextRef("nope")).toBeNull();
    expect(decodeContextRef("")).toBeNull();
  });
});
