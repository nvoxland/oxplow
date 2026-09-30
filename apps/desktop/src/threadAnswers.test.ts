import { expect, test } from "bun:test";

import type { AcpToolCall } from "./api.js";
import { threadRowId } from "./modelIds.js";
import { answerOfTool, answersFromRows } from "./threadAnswers.js";

const call = (over: Partial<AcpToolCall>): AcpToolCall => ({
  id: "t1",
  title: "",
  name: null,
  kind: "other",
  status: "completed",
  locations: [],
  rawInput: null,
  rawOutput: null,
  diffs: [],
  text: [],
  ...over,
});

test("a thread's row id is its number", () => {
  expect(threadRowId("thr12")).toBe(12);
});

test("a finished show_lens call names its answer, whatever the MCP server's prefix", () => {
  const out = JSON.stringify({ answer: "answer:7", title: "Churn", text: "| a |" });
  expect(answerOfTool(call({ name: "mcp__oxplow__show_lens", text: [out] }))).toBe("answer:7");
  expect(
    answerOfTool(call({ title: "mcp__plugin_oxplow_oxplow__show_lens", rawOutput: [{ type: "text", text: out }] })),
  ).toBe("answer:7");
  // Still running, another tool, or a failed call: nothing to show.
  expect(answerOfTool(call({ name: "mcp__oxplow__show_lens", status: "in_progress", text: [out] }))).toBeNull();
  expect(answerOfTool(call({ name: "mcp__oxplow__run_lens", text: [out] }))).toBeNull();
  expect(answerOfTool(call({ name: "mcp__oxplow__show_lens", text: ["refused: DELETE"] }))).toBeNull();
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
