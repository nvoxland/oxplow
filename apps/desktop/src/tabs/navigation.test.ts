import { expect, test } from "bun:test";

import { planNavigation } from "./navigation.js";

const states = {
  str1: { selectedThreadId: "thr1", threads: [{ id: "thr1" }, { id: "thr2" }] },
  str2: { selectedThreadId: "thr5", threads: [{ id: "thr5" }] },
};

test("a ref opens its page, in the shown stream unless one is named", () => {
  const plan = planNavigation({ ref: "file:src/auth.rs" }, states, "str1");
  expect(plan.streamId).toBe("str1");
  expect(plan.threadId).toBe("thr1");
  expect(plan.page?.id).toBe("file:src/auth.rs");
  expect(plan.selectThread).toBe(false);
});

test("the pages a ref can name each resolve", () => {
  for (const ref of ["work_item:oxplow:tsk3", "effort:eff9", "lens:oxplow/tasks", "wiki:overview", "dir:src", "page:git-dashboard"]) {
    expect(planNavigation({ ref }, states, "str1").page?.id).toBe(ref);
  }
});

test("a named stream is switched to, and its selected thread shows the page", () => {
  const plan = planNavigation({ ref: "wiki:overview", stream: "stream:str2" }, states, "str1");
  expect(plan.streamId).toBe("str2");
  expect(plan.threadId).toBe("thr5");
  expect(plan.page?.id).toBe("wiki:overview");
  expect(planNavigation({ ref: "wiki:overview", stream: "str2" }, states, "str1").streamId).toBe("str2");
});

test("a bare stream or thread ref selects it and opens nothing", () => {
  const stream = planNavigation({ ref: "stream:str2" }, states, "str1");
  expect(stream).toMatchObject({ streamId: "str2", threadId: "thr5", selectThread: false });
  expect(stream.page).toBeNull();
  const thread = planNavigation({ ref: "thread:thr2" }, states, "str2");
  expect(thread).toMatchObject({ streamId: "str1", threadId: "thr2", selectThread: true });
  expect(thread.page).toBeNull();
});

test("what can't be reached is refused", () => {
  expect(() => planNavigation({ ref: "nonsense" }, states, "str1")).toThrow();
  expect(() => planNavigation({}, states, "str1")).toThrow(/ref/);
  expect(() => planNavigation({ ref: "wiki:x", stream: "stream:str9" }, states, "str1")).toThrow(/no stream/);
  expect(() => planNavigation({ ref: "thread:thr9" }, states, "str1")).toThrow(/no thread/);
  expect(() => planNavigation({ ref: "stream:str9" }, states, "str1")).toThrow(/no stream/);
  expect(() => planNavigation({ ref: "wiki:x" }, states, null)).toThrow(/no stream/);
});
