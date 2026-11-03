import { describe, expect, test } from "bun:test";
import { selectionUri } from "./agent-context-ref.js";

describe("selectionUri", () => {
  test("the selected lines, dropping a trailing line the selection only touches", () => {
    expect(selectionUri("a.rs", { startLineNumber: 4, endLineNumber: 6, endColumn: 9 })).toBe("oxplow:oxplow/file?path=a.rs#lines=4-6");
    expect(selectionUri("a.rs", { startLineNumber: 4, endLineNumber: 7, endColumn: 1 })).toBe("oxplow:oxplow/file?path=a.rs#lines=4-6");
    expect(selectionUri("a.rs", { startLineNumber: 2, endLineNumber: 2, endColumn: 5 }, "git:abc")).toBe("oxplow:oxplow/file?path=a.rs&rev=git:abc#lines=2-2");
  });
});
