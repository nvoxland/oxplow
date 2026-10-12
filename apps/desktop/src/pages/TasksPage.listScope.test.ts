import { expect, test } from "bun:test";

import { listScope } from "./TasksPage.js";

// The `work_item.list` replacement is given the Tasks page's scope as its
// contract's props: the current thread's row id by default, a picked
// thread's, a stream, or everything.
test("the Tasks page's scope becomes the list contract's props", () => {
  expect(listScope({ kind: "currentThread" }, "thr7")).toEqual({ scope: "thread", thread_id: 7 });
  expect(listScope({ kind: "currentThread" }, null)).toEqual({ scope: "all", thread_id: null });
  expect(listScope({ kind: "thread", streamId: "str2", threadId: "thr3" }, "thr7")).toEqual({ scope: "thread", thread_id: 3 });
  expect(listScope({ kind: "stream", streamId: "str2" }, "thr7")).toEqual({ scope: "stream", thread_id: null });
  expect(listScope({ kind: "all" }, "thr7")).toEqual({ scope: "all", thread_id: null });
});
