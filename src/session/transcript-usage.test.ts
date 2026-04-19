import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { summarizeTurnUsage } from "./transcript-usage.js";

function assistant(timestamp: string, usage: Record<string, number>): string {
  return JSON.stringify({
    type: "assistant",
    timestamp,
    message: { usage },
  });
}

test("sums input/output/cache-read across assistant entries on or after the turn start", () => {
  const text = [
    assistant("2026-04-19T06:00:00Z", { input_tokens: 5, output_tokens: 10, cache_read_input_tokens: 100 }),
    assistant("2026-04-19T07:00:00Z", { input_tokens: 1, output_tokens: 7, cache_read_input_tokens: 20 }),
    assistant("2026-04-19T07:05:00Z", { input_tokens: 2, output_tokens: 3, cache_read_input_tokens: 0 }),
  ].join("\n");

  const usage = summarizeTurnUsage(text, "2026-04-19T07:00:00Z");
  expect(usage).toEqual({ inputTokens: 3, outputTokens: 10, cacheReadInputTokens: 20 });
});

test("skips entries with a timestamp before the turn started", () => {
  const text = assistant("2026-04-19T06:59:59Z", { input_tokens: 999, output_tokens: 999 });
  const usage = summarizeTurnUsage(text, "2026-04-19T07:00:00Z");
  expect(usage).toEqual({ inputTokens: null, outputTokens: null, cacheReadInputTokens: null });
});

test("ignores non-assistant entries (user, tool_result, etc.)", () => {
  const text = [
    JSON.stringify({ type: "user", timestamp: "2026-04-19T07:00:01Z", message: { usage: { input_tokens: 500 } } }),
    JSON.stringify({ type: "tool_result", timestamp: "2026-04-19T07:00:02Z" }),
    assistant("2026-04-19T07:00:03Z", { input_tokens: 4, output_tokens: 8 }),
  ].join("\n");

  const usage = summarizeTurnUsage(text, "2026-04-19T07:00:00Z");
  expect(usage).toEqual({ inputTokens: 4, outputTokens: 8, cacheReadInputTokens: 0 });
});

test("tolerates malformed JSON lines and blank lines without throwing", () => {
  const text = [
    "",
    "not json at all",
    assistant("2026-04-19T07:00:00Z", { input_tokens: 1, output_tokens: 2, cache_read_input_tokens: 3 }),
    "   ",
    "{incomplete",
  ].join("\n");

  const usage = summarizeTurnUsage(text, "2026-04-19T07:00:00Z");
  expect(usage).toEqual({ inputTokens: 1, outputTokens: 2, cacheReadInputTokens: 3 });
});

test("returns all-nulls when no assistant entry falls in the window", () => {
  const usage = summarizeTurnUsage("", "2026-04-19T07:00:00Z");
  expect(usage).toEqual({ inputTokens: null, outputTokens: null, cacheReadInputTokens: null });
});

test("parses a real Claude Code transcript line captured from ~/.claude/projects", () => {
  // Regression: the synthetic-line tests pass arbitrary JSON shapes, so they
  // don't catch drift in Claude Code's actual payload. This fixture is one
  // real assistant line (message.content stripped) from a post-claude-4.7
  // session, plus a couple of non-usage lines to exercise the filter.
  const fixturePath = join(__dirname, "transcript-usage.fixture.jsonl");
  const text = readFileSync(fixturePath, "utf8");
  const usage = summarizeTurnUsage(text, "2026-04-19T06:59:00Z");
  // Both assistant lines (timestamps 06:59:05 and 06:59:10) fall in the
  // window. User + tool_result lines get filtered.
  // input_tokens:  6 +   1 =   7
  // output_tokens: 260 + 645 = 905
  // cache_read:    16397 + 217007 = 233404
  expect(usage).toEqual({ inputTokens: 7, outputTokens: 905, cacheReadInputTokens: 233404 });
});

test("missing usage sub-fields are treated as 0 so one-sided entries still add up", () => {
  const text = [
    assistant("2026-04-19T07:00:01Z", { input_tokens: 1 }),
    assistant("2026-04-19T07:00:02Z", { output_tokens: 2 }),
    assistant("2026-04-19T07:00:03Z", { cache_read_input_tokens: 5 }),
  ].join("\n");

  const usage = summarizeTurnUsage(text, "2026-04-19T07:00:00Z");
  expect(usage).toEqual({ inputTokens: 1, outputTokens: 2, cacheReadInputTokens: 5 });
});
