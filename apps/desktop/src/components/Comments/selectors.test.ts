import { describe, expect, test } from "bun:test";

import { resolveAnchor } from "./anchor.js";
import { anchorInputFromSelectors, buildSelectorsJson, buildTextSelectors } from "./selectors.js";

const TEXT = "The quick brown fox jumps over the lazy dog and runs away.";

describe("buildTextSelectors", () => {
  test("captures exact quote plus prefix/suffix context", () => {
    const start = TEXT.indexOf("brown fox");
    const end = start + "brown fox".length;
    const sels = buildTextSelectors(TEXT, start, end);

    const quote = sels.find((s) => s.type === "TextQuoteSelector");
    const pos = sels.find((s) => s.type === "TextPositionSelector");
    expect(quote).toBeTruthy();
    expect(pos).toBeTruthy();
    if (quote?.type === "TextQuoteSelector") {
      expect(quote.exact).toBe("brown fox");
      expect(quote.prefix).toBe("The quick ");
      // Suffix is capped at CONTEXT_LEN (32) chars.
      expect(quote.suffix).toBe(" jumps over the lazy dog and run");
    }
    if (pos?.type === "TextPositionSelector") {
      expect(pos.start).toBe(start);
      expect(pos.end).toBe(end);
    }
  });
});

describe("anchorInputFromSelectors round-trips through resolveAnchor", () => {
  test("W3C array resolves back to the original span", () => {
    const start = TEXT.indexOf("lazy dog");
    const end = start + "lazy dog".length;
    const json = buildSelectorsJson(TEXT, start, end);

    const input = anchorInputFromSelectors(json, "lazy dog");
    expect(input.quote).toBe("lazy dog");
    expect(input.hintOffset).toBe(start);

    const res = resolveAnchor(TEXT, input);
    expect(res.offset).toBe(start);
    expect(res.confidence).toBe("exact");
  });

  test("quote wins over a stale exact in the stored selector", () => {
    // Stored selectors say "brown fox" but the durable quote is the
    // truth; we anchor on the quote.
    const json = JSON.stringify([
      { type: "TextQuoteSelector", exact: "brown fox", prefix: "The quick ", suffix: " jumps" },
      { type: "TextPositionSelector", start: 4, end: 13 },
    ]);
    const input = anchorInputFromSelectors(json, "quick");
    expect(input.quote).toBe("quick");
  });

  test("reads the legacy Monaco/ProseMirror position object", () => {
    const start = TEXT.indexOf("runs away");
    const legacy = JSON.stringify({
      startLine: 1,
      startColumn: 1,
      prefix: "dog and ",
      suffix: ".",
      textOffset: start,
      approx: false,
    });
    const input = anchorInputFromSelectors(legacy, "runs away");
    expect(input.prefix).toBe("dog and ");
    expect(input.hintOffset).toBe(start);
    const res = resolveAnchor(TEXT, input);
    expect(res.offset).toBe(start);
  });

  test("garbage json degrades to a bare quote anchor", () => {
    const input = anchorInputFromSelectors("not json{", "fox");
    expect(input.quote).toBe("fox");
    expect(input.prefix).toBeUndefined();
    expect(input.hintOffset).toBeUndefined();
  });
});
