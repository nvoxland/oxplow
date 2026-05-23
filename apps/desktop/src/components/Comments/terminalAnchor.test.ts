import { describe, expect, test } from "bun:test";

import {
  buildTerminalSelectorsJson,
  coordToOffset,
  offsetToCoord,
  reanchorInBuffer,
  serializeBuffer,
  type TerminalBufferSelector,
} from "./terminalAnchor.js";

const LINES = ["$ cargo test", "running 3 tests", "test result: FAILED", "error[E0382]: borrow"];

describe("serializeBuffer", () => {
  test("joins lines with newlines and records line starts", () => {
    const buf = serializeBuffer(LINES);
    expect(buf.text).toBe("$ cargo test\nrunning 3 tests\ntest result: FAILED\nerror[E0382]: borrow");
    // "$ cargo test" is 12 chars → next line starts at 13 (after \n).
    expect(buf.lineStarts).toEqual([0, 13, 29, 49]);
  });

  test("empty buffer is empty", () => {
    const buf = serializeBuffer([]);
    expect(buf.text).toBe("");
    expect(buf.lineStarts).toEqual([]);
  });
});

describe("offsetToCoord / coordToOffset round-trip", () => {
  const buf = serializeBuffer(LINES);

  test("start of each line", () => {
    expect(offsetToCoord(buf, 0)).toEqual({ line: 0, col: 0 });
    expect(offsetToCoord(buf, 13)).toEqual({ line: 1, col: 0 });
    expect(offsetToCoord(buf, 29)).toEqual({ line: 2, col: 0 });
  });

  test("mid-line offset", () => {
    // offset 18 is within line 1 ("running 3 tests"), col 5 → "ng 3…"
    expect(offsetToCoord(buf, 18)).toEqual({ line: 1, col: 5 });
    expect(buf.text.slice(18, 18 + 1)).toBe("n");
  });

  test("coordToOffset inverts offsetToCoord", () => {
    for (const off of [0, 5, 13, 20, 35, 49, 60]) {
      const c = offsetToCoord(buf, off);
      expect(coordToOffset(buf, c.line, c.col)).toBe(off);
    }
  });
});

describe("buildTerminalSelectorsJson", () => {
  test("emits W3C text selectors plus a TerminalBufferSelector", () => {
    const buf = serializeBuffer(LINES);
    // Select "result: FAILED" on line 2.
    const start = buf.text.indexOf("result: FAILED");
    const end = start + "result: FAILED".length;
    const json = buildTerminalSelectorsJson(buf, start, end);
    const sels = JSON.parse(json) as Array<Record<string, unknown>>;
    const quote = sels.find((s) => s.type === "TextQuoteSelector");
    expect(quote?.exact).toBe("result: FAILED");
    const term = sels.find((s) => s.type === "TerminalBufferSelector") as
      | TerminalBufferSelector
      | undefined;
    expect(term).toBeDefined();
    expect(term!.startLine).toBe(2);
    expect(term!.startCol).toBe("test ".length); // "test result…" → col 5
    expect(term!.endLine).toBe(2);
  });
});

describe("reanchorInBuffer", () => {
  test("relocates an unchanged quote exactly", () => {
    const buf = serializeBuffer(LINES);
    const start = buf.text.indexOf("result: FAILED");
    const json = buildTerminalSelectorsJson(buf, start, start + "result: FAILED".length);

    const anchor = reanchorInBuffer(buf, json, "result: FAILED");
    expect(anchor).not.toBeNull();
    expect(anchor!.startLine).toBe(2);
    expect(anchor!.startCol).toBe(5);
    expect(anchor!.confidence).toBe("exact");
  });

  test("re-anchors after the buffer scrolled (line index shifted)", () => {
    const original = serializeBuffer(LINES);
    const start = original.text.indexOf("error[E0382]");
    const json = buildTerminalSelectorsJson(
      original,
      start,
      start + "error[E0382]: borrow".length,
    );
    // New output pushed two lines on top; the quote moved down.
    const scrolled = serializeBuffer(["...", "...", ...LINES]);
    const anchor = reanchorInBuffer(scrolled, json, "error[E0382]: borrow");
    expect(anchor).not.toBeNull();
    expect(anchor!.startLine).toBe(5); // was line 3, now line 5
    expect(anchor!.startCol).toBe(0);
  });

  test("returns null when the quote is gone (evicted scrollback)", () => {
    const buf = serializeBuffer(["unrelated output", "nothing matches here"]);
    const anchor = reanchorInBuffer(buf, "[]", "result: FAILED");
    expect(anchor).toBeNull();
  });
});
