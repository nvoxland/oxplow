import { describe, expect, test } from "bun:test";
import { bookmarkedElsewhere, bookmarksFromResult, bookmarksParams, moveBookmarkInput, setBookmarkInput } from "./bookmarks.js";
import type { SqlQueryResult } from "../tauri-bridge/generated/bindings.js";

const result = (rows: (string | null)[][]): SqlQueryResult =>
  ({ columns: ["uri", "label", "scope"], rows, reads: { models: [] } }) as unknown as SqlQueryResult;

describe("bookmarks", () => {
  test("rows become refs; an unparseable ref is skipped", () => {
    const out = bookmarksFromResult(result([
      ["oxplow:oxplow/work_item?id=tsk7&provider=oxplow", "Fix it", "thread"],
      ["", null, "project"],
    ]));
    expect(out).toHaveLength(1);
    expect(out[0]!.ref.id).toBe("oxplow:oxplow/work_item?id=tsk7&provider=oxplow");
    expect(out[0]!.label).toBe("Fix it");
    expect(out[0]!.scope).toBe("thread");
  });

  test("the viewer binds as model row ids", () => {
    expect(bookmarksParams({ threadId: "thr3", streamId: "str2" })).toEqual([3, 2]);
    expect(bookmarksParams({ threadId: null, streamId: null })).toEqual([null, null]);
  });

  test("set names the page, its kind and the thread before the stream", () => {
    const ref = { id: "oxplow:oxplow/page?page=git-dashboard", kind: "git-dashboard", payload: null } as never;
    expect(setBookmarkInput({ threadId: "thr3", streamId: "str2" }, ref, "Git", "project")).toEqual({
      uri: "oxplow:oxplow/page?page=git-dashboard", page_kind: "git-dashboard", label: "Git", scope: "project", thread: "thr3",
    });
    expect(setBookmarkInput({ threadId: null, streamId: "str2" }, ref, null, "stream")).toEqual({
      uri: "oxplow:oxplow/page?page=git-dashboard", page_kind: "git-dashboard", scope: "stream", stream: "str2",
    });
  });
});

const ref = (id: string) => ({ id, kind: "lens", payload: null }) as never;
const mark = (id: string, scope: "thread" | "stream" | "project") => ({ ref: ref(id), label: "L", scope });

describe("a bookmark at other values", () => {
  test("the same page at another view is found; the same view, or another page, is not", () => {
    const marks = [mark("oxplow:oxplow/lens?lens=x/y#a=1", "stream"), mark("oxplow:oxplow/lens?lens=x/z", "thread")];
    expect(bookmarkedElsewhere(marks, ref("oxplow:oxplow/lens?lens=x/y#a=2"))).toBe(marks[0]!);
    expect(bookmarkedElsewhere(marks, ref("oxplow:oxplow/lens?lens=x/y"))).toBe(marks[0]!);
    expect(bookmarkedElsewhere(marks, ref("oxplow:oxplow/lens?lens=x/y#a=1"))).toBeNull();
    expect(bookmarkedElsewhere(marks, ref("oxplow:oxplow/lens?lens=x/w#a=1"))).toBeNull();
  });

  test("updating takes the old one off and stars this view at the same scope, in one sequence", () => {
    const input = moveBookmarkInput({ threadId: "thr3", streamId: "str2" }, mark("oxplow:oxplow/lens?lens=x/y#a=1", "stream"), ref("oxplow:oxplow/lens?lens=x/y#a=2"), "Y");
    expect(input).toEqual({
      calls: [
        { name: "oxplow.bookmark.remove", input: { uri: "oxplow:oxplow/lens?lens=x/y#a=1", thread: "thr3" } },
        { name: "oxplow.bookmark.set", input: { uri: "oxplow:oxplow/lens?lens=x/y#a=2", page_kind: "oxplow/lens", label: "Y", scope: "stream", thread: "thr3" } },
      ],
    });
  });
});
