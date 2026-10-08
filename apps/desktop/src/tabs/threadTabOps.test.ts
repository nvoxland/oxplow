import { expect, test } from "bun:test";

import { streamOfThread, withTab, withoutTab } from "./threadTabOps.js";
import type { TabRef } from "./tabState.js";

const a: TabRef = { id: "file:a.rs", kind: "file", payload: { path: "a.rs" } } as TabRef;

test("a tab is added to and removed from one thread's tabs, others untouched", () => {
  const before = { thr1: [], thr2: [a] };
  const added = withTab(before, "thr1", a);
  expect(added).toEqual({ thr1: [a], thr2: [a] });
  expect(withTab(added, "thr1", a)).toBe(added);
  const removed = withoutTab(added, "thr2", a.id);
  expect(removed).toEqual({ thr1: [a], thr2: [] });
  expect(withoutTab(removed, "thr2", a.id)).toBe(removed);
});

test("a thread's stream is found by the window's thread lists", () => {
  const states = { str1: { threads: [{ id: "thr1" }] }, str2: { threads: [{ id: "thr5" }] } };
  expect(streamOfThread(states, "thr5")).toBe("str2");
  expect(streamOfThread(states, "thr9")).toBeNull();
});
