import { describe, expect, test } from "bun:test";

import { formatTokens, tokenTotalsSummary } from "./tokens.js";

describe("formatTokens", () => {
  test("renders small counts verbatim", () => {
    expect(formatTokens(0)).toBe("0");
    expect(formatTokens(936)).toBe("936");
    expect(formatTokens(999)).toBe("999");
  });

  test("renders thousands with a trimmed decimal", () => {
    expect(formatTokens(1000)).toBe("1K");
    expect(formatTokens(1234)).toBe("1.2K");
    expect(formatTokens(12000)).toBe("12K");
    expect(formatTokens(12500)).toBe("12.5K");
  });

  test("renders millions", () => {
    expect(formatTokens(1_000_000)).toBe("1M");
    expect(formatTokens(1_200_000)).toBe("1.2M");
    expect(formatTokens(37_420_000)).toBe("37.4M");
  });

  test("handles negatives", () => {
    expect(formatTokens(-1500)).toBe("-1.5K");
  });
});

describe("tokenTotalsSummary", () => {
  test("summarizes total tokens + turns", () => {
    expect(
      tokenTotalsSummary({
        input_tokens: 1,
        output_tokens: 2,
        cache_creation_input_tokens: 3,
        cache_read_input_tokens: 4,
        total_tokens: 1_200_000,
        message_count: 9,
        turns: 8,
      }),
    ).toBe("1.2M tokens · 8 turns");
  });

  test("singular turn", () => {
    expect(
      tokenTotalsSummary({
        input_tokens: 100,
        output_tokens: 20,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
        total_tokens: 120,
        message_count: 1,
        turns: 1,
      }),
    ).toBe("120 tokens · 1 turn");
  });
});
