import { describe, expect, test } from "bun:test";
import {
  EditorFocusStore,
  formatEditorFocusForAgent,
  type EditorFocusState,
} from "./editor-focus.js";

function baseState(overrides: Partial<EditorFocusState> = {}): EditorFocusState {
  return {
    activeFile: null,
    caret: null,
    selection: null,
    openFiles: [],
    updatedAt: "2026-04-18T00:00:00.000Z",
    ...overrides,
  };
}

describe("formatEditorFocusForAgent", () => {
  test("returns null when nothing is set", () => {
    expect(formatEditorFocusForAgent(null)).toBeNull();
    expect(formatEditorFocusForAgent(baseState())).toBeNull();
  });

  test("emits active file, caret, and open tabs when present", () => {
    const formatted = formatEditorFocusForAgent(
      baseState({
        activeFile: "src/foo.ts",
        caret: { line: 42, column: 18 },
        openFiles: [
          { path: "src/foo.ts", dirty: true },
          { path: "src/bar.ts", dirty: false },
        ],
      }),
    );
    expect(formatted).not.toBeNull();
    expect(formatted).toContain("<editor-context>");
    expect(formatted).toContain("Active file: src/foo.ts");
    expect(formatted).toContain("Caret: line 42, col 18");
    expect(formatted).toContain("Open tabs: src/foo.ts (dirty), src/bar.ts");
    expect(formatted).toContain("</editor-context>");
  });

  test("emits selection with fenced code block and 1-based range header", () => {
    const formatted = formatEditorFocusForAgent(
      baseState({
        activeFile: "src/foo.ts",
        selection: {
          startLine: 10,
          startColumn: 1,
          endLine: 12,
          endColumn: 5,
          text: "const a = 1;\nconst b = 2;\nconst",
        },
      }),
    );
    expect(formatted).toContain("Selection (src/foo.ts:10-12):");
    expect(formatted).toContain("```\nconst a = 1;\nconst b = 2;\nconst\n```");
  });

  test("omits caret section when a selection is present", () => {
    const formatted = formatEditorFocusForAgent(
      baseState({
        activeFile: "a.ts",
        caret: { line: 1, column: 1 },
        selection: {
          startLine: 1,
          startColumn: 1,
          endLine: 1,
          endColumn: 5,
          text: "abcd",
        },
      }),
    );
    expect(formatted).not.toContain("Caret:");
    expect(formatted).toContain("Selection");
  });

  test("truncates selections over 200 lines and reports remaining", () => {
    const lines = Array.from({ length: 250 }, (_, i) => `line${i + 1}`);
    const formatted = formatEditorFocusForAgent(
      baseState({
        activeFile: "big.ts",
        selection: {
          startLine: 1,
          startColumn: 1,
          endLine: 250,
          endColumn: 7,
          text: lines.join("\n"),
        },
      }),
    );
    expect(formatted).toContain("line1\n");
    expect(formatted).toContain("line200\n");
    expect(formatted).not.toContain("line201");
    expect(formatted).toContain("[truncated 50 more lines]");
  });

  test("caps open tab list at 30 entries", () => {
    const files = Array.from({ length: 35 }, (_, i) => ({ path: `f${i}.ts`, dirty: false }));
    const formatted = formatEditorFocusForAgent(
      baseState({ activeFile: "f0.ts", openFiles: files }),
    );
    expect(formatted).toContain("f0.ts");
    expect(formatted).toContain("f29.ts");
    expect(formatted).not.toContain("f30.ts");
    expect(formatted).toContain("(+5 more)");
  });
});

describe("EditorFocusStore", () => {
  test("isolates state per stream id", () => {
    const store = new EditorFocusStore();
    store.set("s1", baseState({ activeFile: "a.ts" }));
    store.set("s2", baseState({ activeFile: "b.ts" }));
    expect(store.get("s1")?.activeFile).toBe("a.ts");
    expect(store.get("s2")?.activeFile).toBe("b.ts");
  });

  test("clear removes just the given stream", () => {
    const store = new EditorFocusStore();
    store.set("s1", baseState({ activeFile: "a.ts" }));
    store.set("s2", baseState({ activeFile: "b.ts" }));
    store.clear("s1");
    expect(store.get("s1")).toBeNull();
    expect(store.get("s2")?.activeFile).toBe("b.ts");
  });
});
