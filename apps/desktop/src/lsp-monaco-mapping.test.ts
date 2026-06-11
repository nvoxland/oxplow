import { describe, expect, test } from "bun:test";

import {
  completionResultToMonacoList,
  definitionResultToMonacoLocations,
  documentSymbolsToMonaco,
  lspCompletionKindToMonaco,
  lspSymbolKindToMonaco,
  markersToLspDiagnostics,
  normalizeHoverContents,
  normalizeWorkspaceEdit,
  toMonacoRange,
  toMonacoWorkspaceEdit,
} from "./lsp-monaco-mapping.js";

/// Minimal monaco fake: Range/Uri constructors + the enum tables the
/// mapping functions consult.
class FakeRange {
  constructor(
    public startLineNumber: number,
    public startColumn: number,
    public endLineNumber: number,
    public endColumn: number,
  ) {}
}

const fakeMonaco = {
  Range: FakeRange,
  Uri: { parse: (s: string) => ({ toString: () => s, path: s }) },
  languages: {
    CompletionItemKind: {
      Method: 0,
      Function: 1,
      Field: 3,
      Variable: 4,
      Class: 5,
      Keyword: 17,
      Text: 18,
      Snippet: 27,
    },
  },
};

describe("toMonacoRange", () => {
  test("converts 0-based LSP to 1-based Monaco", () => {
    const r = toMonacoRange(fakeMonaco, {
      start: { line: 2, character: 4 },
      end: { line: 2, character: 9 },
    });
    expect([r.startLineNumber, r.startColumn, r.endLineNumber, r.endColumn]).toEqual([3, 5, 3, 10]);
  });
});

describe("completion mapping", () => {
  const defaultRange = new FakeRange(1, 1, 1, 4);

  test("kind table maps LSP kinds to monaco enum values, defaulting to Text", () => {
    expect(lspCompletionKindToMonaco(fakeMonaco, 2)).toBe(0); // Method
    expect(lspCompletionKindToMonaco(fakeMonaco, 3)).toBe(1); // Function
    expect(lspCompletionKindToMonaco(fakeMonaco, 14)).toBe(17); // Keyword
    expect(lspCompletionKindToMonaco(fakeMonaco, undefined)).toBe(18); // Text fallback
    expect(lspCompletionKindToMonaco(fakeMonaco, 99)).toBe(18);
  });

  test("handles CompletionItem[] with insertText fallback chain", () => {
    const { suggestions } = completionResultToMonacoList(
      fakeMonaco,
      [
        { label: "fooBar", kind: 3 },
        { label: "baz", insertText: "baz()" },
        { label: { label: "qux" }, textEdit: { newText: "quux", range: { start: { line: 0, character: 0 }, end: { line: 0, character: 3 } } } },
      ],
      defaultRange,
    );
    expect(suggestions).toHaveLength(3);
    expect(suggestions[0]).toMatchObject({ label: "fooBar", insertText: "fooBar", kind: 1 });
    expect(suggestions[0].range).toBe(defaultRange);
    expect(suggestions[1].insertText).toBe("baz()");
    expect(suggestions[2].insertText).toBe("quux");
    expect(suggestions[2].range.startColumn).toBe(1);
    expect(suggestions[2].range.endColumn).toBe(4);
  });

  test("handles CompletionList with isIncomplete and InsertReplaceEdit", () => {
    const { suggestions, incomplete } = completionResultToMonacoList(
      fakeMonaco,
      {
        isIncomplete: true,
        items: [
          {
            label: "x",
            textEdit: {
              newText: "x2",
              insert: { start: { line: 0, character: 0 }, end: { line: 0, character: 1 } },
              replace: { start: { line: 0, character: 0 }, end: { line: 0, character: 5 } },
            },
          },
        ],
      },
      defaultRange,
    );
    expect(incomplete).toBe(true);
    expect(suggestions[0].range.insert.endColumn).toBe(2);
    expect(suggestions[0].range.replace.endColumn).toBe(6);
  });

  test("null result and junk items yield no suggestions", () => {
    expect(completionResultToMonacoList(fakeMonaco, null, defaultRange).suggestions).toEqual([]);
    expect(
      completionResultToMonacoList(fakeMonaco, [null, 42, {}], defaultRange).suggestions,
    ).toEqual([]);
  });

  test("documentation supports string and markup forms", () => {
    const { suggestions } = completionResultToMonacoList(
      fakeMonaco,
      [
        { label: "a", documentation: "plain" },
        { label: "b", documentation: { kind: "markdown", value: "**md**" } },
      ],
      defaultRange,
    );
    expect(suggestions[0].documentation).toBe("plain");
    expect(suggestions[1].documentation).toEqual({ value: "**md**" });
  });
});

describe("normalizeWorkspaceEdit", () => {
  const textEdit = {
    range: { start: { line: 0, character: 0 }, end: { line: 0, character: 3 } },
    newText: "new",
  };

  test("flattens changes map", () => {
    const out = normalizeWorkspaceEdit({ changes: { "file:///a.ts": [textEdit] } });
    expect(out.files).toEqual([{ uri: "file:///a.ts", edits: [textEdit] }]);
    expect(out.skippedFileOps).toBe(0);
  });

  test("flattens documentChanges and counts file operations", () => {
    const out = normalizeWorkspaceEdit({
      documentChanges: [
        { textDocument: { uri: "file:///a.ts", version: 3 }, edits: [textEdit] },
        { kind: "rename", oldUri: "file:///a.ts", newUri: "file:///b.ts" },
        { kind: "create", uri: "file:///c.ts" },
      ],
    });
    expect(out.files).toHaveLength(1);
    expect(out.files[0].uri).toBe("file:///a.ts");
    expect(out.skippedFileOps).toBe(2);
  });

  test("merges changes + documentChanges for the same uri", () => {
    const out = normalizeWorkspaceEdit({
      changes: { "file:///a.ts": [textEdit] },
      documentChanges: [{ textDocument: { uri: "file:///a.ts" }, edits: [textEdit] }],
    });
    expect(out.files).toHaveLength(1);
    expect(out.files[0].edits).toHaveLength(2);
  });

  test("null/garbage input yields empty edit", () => {
    expect(normalizeWorkspaceEdit(null).files).toEqual([]);
    expect(normalizeWorkspaceEdit("x").files).toEqual([]);
  });

  test("toMonacoWorkspaceEdit produces resource/textEdit rows", () => {
    const normalized = normalizeWorkspaceEdit({ changes: { "file:///a.ts": [textEdit] } });
    const monacoEdit = toMonacoWorkspaceEdit(fakeMonaco, normalized);
    expect(monacoEdit.edits).toHaveLength(1);
    expect(monacoEdit.edits[0].resource.toString()).toBe("file:///a.ts");
    expect(monacoEdit.edits[0].textEdit.text).toBe("new");
    expect(monacoEdit.edits[0].textEdit.range.startLineNumber).toBe(1);
  });
});

describe("markersToLspDiagnostics", () => {
  test("converts 1-based markers to 0-based LSP diagnostics with severity map", () => {
    const out = markersToLspDiagnostics([
      {
        startLineNumber: 3,
        startColumn: 5,
        endLineNumber: 3,
        endColumn: 9,
        message: "boom",
        severity: 8,
        source: "ts",
        code: "2304",
      },
      { startLineNumber: 1, startColumn: 1, message: "warn", severity: 4 },
    ]);
    expect(out).toHaveLength(2);
    expect(out[0]).toMatchObject({
      range: { start: { line: 2, character: 4 }, end: { line: 2, character: 8 } },
      message: "boom",
      severity: 1,
      source: "ts",
      code: "2304",
    });
    expect((out[1] as { severity: number }).severity).toBe(2);
  });

  test("non-array and junk rows are dropped", () => {
    expect(markersToLspDiagnostics(undefined as unknown as unknown[])).toEqual([]);
    expect(markersToLspDiagnostics([null, {}])).toEqual([]);
  });
});

describe("document symbols", () => {
  test("symbol kinds shift from 1-based LSP to 0-based Monaco", () => {
    expect(lspSymbolKindToMonaco(1)).toBe(0); // File
    expect(lspSymbolKindToMonaco(12)).toBe(11); // Function
    expect(lspSymbolKindToMonaco(undefined)).toBe(0);
  });

  test("hierarchical DocumentSymbol[] maps children recursively", () => {
    const out = documentSymbolsToMonaco(fakeMonaco, [
      {
        name: "MyClass",
        kind: 5,
        range: { start: { line: 0, character: 0 }, end: { line: 10, character: 0 } },
        selectionRange: { start: { line: 0, character: 6 }, end: { line: 0, character: 13 } },
        children: [
          {
            name: "method",
            kind: 6,
            range: { start: { line: 1, character: 2 }, end: { line: 3, character: 2 } },
            selectionRange: { start: { line: 1, character: 2 }, end: { line: 1, character: 8 } },
          },
        ],
      },
    ]);
    expect(out).toHaveLength(1);
    expect(out[0].name).toBe("MyClass");
    expect(out[0].kind).toBe(4);
    expect(out[0].children).toHaveLength(1);
    expect(out[0].children[0].name).toBe("method");
    expect(out[0].selectionRange.startColumn).toBe(7);
  });

  test("flat SymbolInformation[] maps location.range", () => {
    const out = documentSymbolsToMonaco(fakeMonaco, [
      {
        name: "fn_a",
        kind: 12,
        location: {
          uri: "file:///a.rs",
          range: { start: { line: 4, character: 0 }, end: { line: 6, character: 1 } },
        },
      },
    ]);
    expect(out).toHaveLength(1);
    expect(out[0].range.startLineNumber).toBe(5);
    expect(out[0].selectionRange.startLineNumber).toBe(5);
    expect(out[0].children).toEqual([]);
  });

  test("non-array input and nameless symbols are dropped", () => {
    expect(documentSymbolsToMonaco(fakeMonaco, null)).toEqual([]);
    expect(documentSymbolsToMonaco(fakeMonaco, [{ kind: 5 }])).toEqual([]);
  });
});

describe("hover + definition passthroughs", () => {
  test("normalizeHoverContents handles string / markup / code-block forms", () => {
    expect(
      normalizeHoverContents(["plain", { value: "markup" }, { language: "ts", value: "x" }]),
    ).toEqual([{ value: "plain" }, { value: "markup" }, { value: "```ts\nx\n```" }]);
  });

  test("definitionResultToMonacoLocations handles single and LocationLink forms", () => {
    const single = definitionResultToMonacoLocations(fakeMonaco, {
      uri: "file:///a.ts",
      range: { start: { line: 0, character: 0 }, end: { line: 0, character: 1 } },
    });
    expect(single).toHaveLength(1);
    const links = definitionResultToMonacoLocations(fakeMonaco, [
      {
        targetUri: "file:///b.ts",
        targetRange: { start: { line: 1, character: 0 }, end: { line: 2, character: 0 } },
        targetSelectionRange: { start: { line: 1, character: 4 }, end: { line: 1, character: 9 } },
      },
    ]);
    expect(links[0].uri.toString()).toBe("file:///b.ts");
    expect(links[0].range.startColumn).toBe(5); // selection range wins
  });
});
