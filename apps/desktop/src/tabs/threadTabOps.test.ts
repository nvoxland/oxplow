import { expect, test } from "bun:test";

import { streamOfThread } from "./threadTabOps.js";

test("a thread's stream is found by the window's thread lists", () => {
  const states = { str1: { threads: [{ id: "thr1" }] }, str2: { threads: [{ id: "thr5" }] } };
  expect(streamOfThread(states, "thr5")).toBe("str2");
  expect(streamOfThread(states, "thr9")).toBeNull();
});
