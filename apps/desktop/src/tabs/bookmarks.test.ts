import { describe, expect, test } from "bun:test";
import { bookmarksFromResult, bookmarksParams, setBookmarkInput } from "./bookmarks.js";
import type { SqlQueryResult } from "../tauri-bridge/generated/bindings.js";

const result = (rows: (string | null)[][]): SqlQueryResult =>
  ({ columns: ["ref", "label", "scope"], rows, reads: { models: [] } }) as unknown as SqlQueryResult;

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
      ref: "oxplow:oxplow/page?page=git-dashboard", page_kind: "git-dashboard", label: "Git", scope: "project", thread: "oxplow:oxplow/thread?id=thr3",
    });
    expect(setBookmarkInput({ threadId: null, streamId: "str2" }, ref, null, "stream")).toEqual({
      ref: "oxplow:oxplow/page?page=git-dashboard", page_kind: "git-dashboard", scope: "stream", stream: "oxplow:oxplow/stream?id=str2",
    });
  });
});
