import { expect, test } from "bun:test";

import { threadRowId } from "./modelIds.js";
import { answersFromRows } from "./threadAnswers.js";

test("a thread's row id is its number", () => {
  expect(threadRowId("thr12")).toBe(12);
});

test("answers read newest first, with the lens they show or were kept as", () => {
  expect(
    answersFromRows([
      ["answer:2", "Churn", null, "my-lenses/churn"],
      ["answer:1", "Open tasks", "review/waiting", null],
    ]),
  ).toEqual([
    { ref: "answer:2", title: "Churn", lens: null, keptLens: "my-lenses/churn" },
    { ref: "answer:1", title: "Open tasks", lens: "review/waiting", keptLens: null },
  ]);
});
