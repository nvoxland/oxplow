import { describe, expect, test } from "bun:test";
import { editorContextEntry, selectionUri } from "./agent-context-ref.js";

describe("selectionUri", () => {
  test("the selected lines, dropping a trailing line the selection only touches", () => {
    expect(selectionUri("a.rs", { startLineNumber: 4, endLineNumber: 6, endColumn: 9 })).toBe("oxplow:oxplow/file?path=a.rs#lines=4-6");
    expect(selectionUri("a.rs", { startLineNumber: 4, endLineNumber: 7, endColumn: 1 })).toBe("oxplow:oxplow/file?path=a.rs#lines=4-6");
    expect(selectionUri("a.rs", { startLineNumber: 2, endLineNumber: 2, endColumn: 5 }, "git:abc")).toBe("oxplow:oxplow/file?path=a.rs&rev=git:abc#lines=2-2");
  });
});

// The editor's menu entry: a selection goes in as its lines' text; with
// none, the whole file goes in by its URI.
test("editorContextEntry is the selection's lines, else the file", () => {
  const sel = { startLineNumber: 3, endLineNumber: 4, endColumn: 5, isEmpty: () => false };
  expect(editorContextEntry("src/a.rs", sel, "fn x() {}")).toEqual({
    label: "Add Selection to Context",
    what: { uri: "oxplow:oxplow/file?path=src/a.rs#lines=3-4", text: "fn x() {}" },
  });
  expect(editorContextEntry("src/a.rs", null, null)).toEqual({ label: "Add File to Context", what: { uri: "oxplow:oxplow/file?path=src/a.rs", label: "a.rs" } });
  expect(editorContextEntry("src/a.rs", { ...sel, isEmpty: () => true }, "")).toEqual({ label: "Add File to Context", what: { uri: "oxplow:oxplow/file?path=src/a.rs", label: "a.rs" } });
});
