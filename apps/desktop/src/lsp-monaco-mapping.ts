/// Pure LSP ↔ Monaco shape conversions for the editor's LSP
/// providers. Every function takes `monaco` as a parameter (only enum
/// tables + Uri/Range constructors are used) so the logic unit-tests
/// with a tiny fake — no Monaco mount needed.

import type { Stream } from "./api.js";
import { type EditorNavigationTarget, toEditorNavigationTarget } from "./lsp.js";

/* eslint-disable @typescript-eslint/no-explicit-any */

export interface LspRange {
  start?: { line?: number; character?: number };
  end?: { line?: number; character?: number };
}

/// LSP ranges are 0-based; Monaco's are 1-based.
export function toMonacoRange(monaco: any, range: unknown): any {
  const candidate = range as LspRange;
  return new monaco.Range(
    (candidate.start?.line ?? 0) + 1,
    (candidate.start?.character ?? 0) + 1,
    (candidate.end?.line ?? candidate.start?.line ?? 0) + 1,
    (candidate.end?.character ?? candidate.start?.character ?? 0) + 1,
  );
}

/// Location | LocationLink → Monaco location ({ uri, range }).
export function referenceToMonacoLocation(monaco: any, item: unknown): any | null {
  if (!item || typeof item !== "object") return null;
  const candidate = item as {
    uri?: string;
    targetUri?: string;
    range?: unknown;
    targetSelectionRange?: unknown;
    targetRange?: unknown;
  };
  const uri = candidate.targetUri ?? candidate.uri;
  const range = candidate.targetSelectionRange ?? candidate.targetRange ?? candidate.range;
  if (!uri || !range) return null;
  return {
    uri: monaco.Uri.parse(uri),
    range: toMonacoRange(monaco, range),
  };
}

export function definitionResultToMonacoLocations(monaco: any, result: unknown): any[] {
  const locations = Array.isArray(result) ? result : result ? [result] : [];
  return locations
    .map((item) => referenceToMonacoLocation(monaco, item))
    .filter(Boolean);
}

export function normalizeDefinitionTarget(
  stream: Stream,
  result: unknown,
): EditorNavigationTarget | null {
  const locations = Array.isArray(result) ? result : result ? [result] : [];
  for (const location of locations) {
    if (!location || typeof location !== "object") continue;
    const candidate = location as {
      uri?: string;
      targetUri?: string;
      range?: LspRange;
      targetSelectionRange?: LspRange;
      targetRange?: LspRange;
    };
    if (candidate.targetUri) {
      const target = toEditorNavigationTarget(
        stream,
        candidate.targetUri,
        candidate.targetSelectionRange ?? candidate.targetRange,
      );
      if (target) return target;
    }
    if (candidate.uri) {
      const target = toEditorNavigationTarget(stream, candidate.uri, candidate.range);
      if (target) return target;
    }
  }
  return null;
}

export function normalizeHoverContents(contents: unknown): { value: string }[] {
  const values = Array.isArray(contents) ? contents : [contents];
  return values.flatMap((item) => {
    if (!item) return [];
    if (typeof item === "string") return [{ value: item }];
    // MarkedString {language, value} renders as a fenced code block —
    // check before the generic {value} branch, which would shadow it.
    if (typeof item === "object" && "language" in item && "value" in item) {
      const markup = item as { language?: unknown; value?: unknown };
      if (typeof markup.value === "string") {
        return [{ value: `\`\`\`${typeof markup.language === "string" ? markup.language : ""}\n${markup.value}\n\`\`\`` }];
      }
    }
    if (typeof item === "object" && "value" in item && typeof (item as { value?: unknown }).value === "string") {
      return [{ value: (item as { value: string }).value }];
    }
    return [];
  });
}

// ---------------------------------------------------------------------
// Completion
// ---------------------------------------------------------------------

/// LSP CompletionItemKind (1-based) → Monaco CompletionItemKind enum
/// name. The two enums list the same concepts in different orders, so
/// the mapping is by name, resolved against the live enum at call time.
const COMPLETION_KIND_NAMES: Record<number, string> = {
  1: "Text",
  2: "Method",
  3: "Function",
  4: "Constructor",
  5: "Field",
  6: "Variable",
  7: "Class",
  8: "Interface",
  9: "Module",
  10: "Property",
  11: "Unit",
  12: "Value",
  13: "Enum",
  14: "Keyword",
  15: "Snippet",
  16: "Color",
  17: "File",
  18: "Reference",
  19: "Folder",
  20: "EnumMember",
  21: "Constant",
  22: "Struct",
  23: "Event",
  24: "Operator",
  25: "TypeParameter",
};

export function lspCompletionKindToMonaco(monaco: any, kind: unknown): number {
  const name = typeof kind === "number" ? COMPLETION_KIND_NAMES[kind] : undefined;
  const table = monaco.languages.CompletionItemKind;
  return (name && table[name]) ?? table.Text ?? 0;
}

function documentationValue(doc: unknown): string | { value: string } | undefined {
  if (typeof doc === "string") return doc;
  if (doc && typeof doc === "object" && typeof (doc as { value?: unknown }).value === "string") {
    return { value: (doc as { value: string }).value };
  }
  return undefined;
}

/// `CompletionList | CompletionItem[] | null` → Monaco suggestions.
/// `defaultRange` is the word range at the cursor (Monaco requires a
/// range per suggestion; items without a textEdit use it).
export function completionResultToMonacoList(
  monaco: any,
  result: unknown,
  defaultRange: any,
): { suggestions: any[]; incomplete: boolean } {
  const items: unknown[] = Array.isArray(result)
    ? result
    : result && typeof result === "object" && Array.isArray((result as { items?: unknown }).items)
      ? ((result as { items: unknown[] }).items)
      : [];
  const incomplete =
    !!result &&
    typeof result === "object" &&
    (result as { isIncomplete?: boolean }).isIncomplete === true;

  const suggestions = items.flatMap((raw) => {
    if (!raw || typeof raw !== "object") return [];
    const item = raw as {
      label?: unknown;
      kind?: unknown;
      detail?: string;
      documentation?: unknown;
      sortText?: string;
      filterText?: string;
      preselect?: boolean;
      insertText?: string;
      commitCharacters?: string[];
      textEdit?: {
        newText?: string;
        range?: LspRange;
        insert?: LspRange;
        replace?: LspRange;
      };
    };
    const label =
      typeof item.label === "string"
        ? item.label
        : typeof (item.label as { label?: unknown })?.label === "string"
          ? (item.label as { label: string }).label
          : null;
    if (!label) return [];

    let range = defaultRange;
    if (item.textEdit?.range) {
      range = toMonacoRange(monaco, item.textEdit.range);
    } else if (item.textEdit?.insert && item.textEdit?.replace) {
      // InsertReplaceEdit — Monaco models this as {insert, replace}.
      range = {
        insert: toMonacoRange(monaco, item.textEdit.insert),
        replace: toMonacoRange(monaco, item.textEdit.replace),
      };
    }

    return [
      {
        label,
        kind: lspCompletionKindToMonaco(monaco, item.kind),
        insertText: item.textEdit?.newText ?? item.insertText ?? label,
        range,
        detail: item.detail,
        documentation: documentationValue(item.documentation),
        sortText: item.sortText,
        filterText: item.filterText,
        preselect: item.preselect,
        commitCharacters: item.commitCharacters,
      },
    ];
  });

  return { suggestions, incomplete };
}

// ---------------------------------------------------------------------
// Workspace edits (rename, code actions)
// ---------------------------------------------------------------------

export interface NormalizedFileEdits {
  uri: string;
  edits: { range: LspRange; newText: string }[];
}

export interface NormalizedWorkspaceEdit {
  files: NormalizedFileEdits[];
  /// Count of documentChanges entries we can't apply (create/rename/
  /// delete file operations) — surfaced to the user, not silently
  /// dropped.
  skippedFileOps: number;
}

/// LSP WorkspaceEdit (`changes` and/or `documentChanges`) → a flat
/// per-uri list of text edits. File create/rename/delete operations
/// are counted but not returned.
export function normalizeWorkspaceEdit(edit: unknown): NormalizedWorkspaceEdit {
  const out = new Map<string, { range: LspRange; newText: string }[]>();
  let skippedFileOps = 0;
  if (!edit || typeof edit !== "object") return { files: [], skippedFileOps };
  const e = edit as {
    changes?: Record<string, { range?: LspRange; newText?: string }[]>;
    documentChanges?: unknown[];
  };

  const push = (uri: string, edits: { range?: LspRange; newText?: string }[] | undefined) => {
    if (!Array.isArray(edits)) return;
    const list = out.get(uri) ?? [];
    for (const te of edits) {
      if (!te?.range || typeof te.newText !== "string") continue;
      list.push({ range: te.range, newText: te.newText });
    }
    if (list.length) out.set(uri, list);
  };

  if (e.changes && typeof e.changes === "object") {
    for (const [uri, edits] of Object.entries(e.changes)) push(uri, edits);
  }
  if (Array.isArray(e.documentChanges)) {
    for (const change of e.documentChanges) {
      if (!change || typeof change !== "object") continue;
      const c = change as {
        kind?: string;
        textDocument?: { uri?: string };
        edits?: { range?: LspRange; newText?: string }[];
      };
      if (typeof c.kind === "string") {
        // CreateFile / RenameFile / DeleteFile.
        skippedFileOps += 1;
        continue;
      }
      if (c.textDocument?.uri) push(c.textDocument.uri, c.edits);
    }
  }

  return {
    files: [...out.entries()].map(([uri, edits]) => ({ uri, edits })),
    skippedFileOps,
  };
}

/// Normalized edit → the WorkspaceEdit shape Monaco's bulk-edit service
/// consumes ({ edits: [{ resource, textEdit, versionId }] }).
export function toMonacoWorkspaceEdit(monaco: any, normalized: NormalizedWorkspaceEdit): any {
  return {
    edits: normalized.files.flatMap((file) =>
      file.edits.map((te) => ({
        resource: monaco.Uri.parse(file.uri),
        textEdit: { range: toMonacoRange(monaco, te.range), text: te.newText },
        versionId: undefined,
      })),
    ),
  };
}

// ---------------------------------------------------------------------
// Code actions
// ---------------------------------------------------------------------

/// Monaco marker severity → LSP DiagnosticSeverity.
const MARKER_SEVERITY_TO_LSP: Record<number, number> = {
  8: 1, // Error
  4: 2, // Warning
  2: 3, // Info
  1: 4, // Hint
};

/// Monaco markers (from CodeActionContext) → LSP Diagnostic[] for
/// `textDocument/codeAction`'s context.
export function markersToLspDiagnostics(markers: unknown[]): unknown[] {
  if (!Array.isArray(markers)) return [];
  return markers.flatMap((raw) => {
    if (!raw || typeof raw !== "object") return [];
    const m = raw as {
      startLineNumber?: number;
      startColumn?: number;
      endLineNumber?: number;
      endColumn?: number;
      message?: string;
      severity?: number;
      source?: string;
      code?: unknown;
    };
    if (m.startLineNumber == null) return [];
    return [
      {
        range: {
          start: { line: m.startLineNumber - 1, character: (m.startColumn ?? 1) - 1 },
          end: {
            line: (m.endLineNumber ?? m.startLineNumber) - 1,
            character: (m.endColumn ?? m.startColumn ?? 1) - 1,
          },
        },
        message: m.message ?? "",
        severity: MARKER_SEVERITY_TO_LSP[m.severity ?? 8] ?? 1,
        source: m.source,
        code: typeof m.code === "string" || typeof m.code === "number" ? m.code : undefined,
      },
    ];
  });
}

// ---------------------------------------------------------------------
// Document symbols
// ---------------------------------------------------------------------

/// LSP SymbolKind is 1-based; Monaco's SymbolKind enum lists the same
/// concepts 0-based in the same order.
export function lspSymbolKindToMonaco(kind: unknown): number {
  return typeof kind === "number" && kind >= 1 && kind <= 26 ? kind - 1 : 0;
}

/// `DocumentSymbol[]` (hierarchical) or `SymbolInformation[]` (flat) →
/// Monaco DocumentSymbol[].
export function documentSymbolsToMonaco(monaco: any, result: unknown): any[] {
  if (!Array.isArray(result)) return [];
  return result.flatMap((raw) => {
    const sym = mapSymbol(monaco, raw);
    return sym ? [sym] : [];
  });
}

function mapSymbol(monaco: any, raw: unknown): any | null {
  if (!raw || typeof raw !== "object") return null;
  const s = raw as {
    name?: string;
    detail?: string;
    kind?: number;
    range?: LspRange;
    selectionRange?: LspRange;
    children?: unknown[];
    location?: { range?: LspRange };
  };
  if (typeof s.name !== "string") return null;
  // Hierarchical DocumentSymbol has `range`; flat SymbolInformation
  // nests it under `location`.
  const range = s.range ?? s.location?.range;
  if (!range) return null;
  const monacoRange = toMonacoRange(monaco, range);
  return {
    name: s.name,
    detail: s.detail ?? "",
    kind: lspSymbolKindToMonaco(s.kind),
    tags: [],
    range: monacoRange,
    selectionRange: s.selectionRange ? toMonacoRange(monaco, s.selectionRange) : monacoRange,
    children: Array.isArray(s.children)
      ? s.children.map((c) => mapSymbol(monaco, c)).filter(Boolean)
      : [],
  };
}
