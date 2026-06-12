/// Monaco LSP provider registration, extracted from EditorPane.
///
/// Registration strategy: providers register ONCE, for every language
/// id in `editor-language.ts`'s extension map, and each callback first
/// checks `hasServer(languageId)` — returning null when no server is
/// configured. Re-registering providers when the server list changes
/// would race Monaco's open widgets; inert providers are cheap.
///
/// Each provider flushes pending didChange for the model's document
/// before issuing its request (`flushDoc`) so the server never answers
/// against stale text — the didChange debounce makes this load-bearing.

import { allKnownLanguageIds } from "../editor-language.js";
import type { LspClient } from "../lsp.js";
import {
  completionResultToMonacoList,
  definitionResultToMonacoLocations,
  documentSymbolsToMonaco,
  markersToLspDiagnostics,
  normalizeHoverContents,
  normalizeWorkspaceEdit,
  referenceToMonacoLocation,
  toMonacoRange,
  toMonacoWorkspaceEdit,
  type NormalizedWorkspaceEdit,
} from "../lsp-monaco-mapping.js";
import {
  partitionByOpenModel,
  type ApplyWorkspaceEditResult,
} from "../lsp-workspace-edit.js";

/* eslint-disable @typescript-eslint/no-explicit-any */

export interface LspProviderDeps {
  hasServer(languageId: string): boolean;
  getClient(languageId: string): LspClient;
  /// Flush pending (debounced) didChange for the model's document.
  flushDoc(model: any): void;
  /// Surface a status-banner message (skipped file ops, etc.).
  setStatus(message: string | null): void;
  /// Apply a normalized workspace edit across open models AND non-open
  /// files (read-modify-write through the workspace file IPC).
  applyEdits(normalized: NormalizedWorkspaceEdit): Promise<ApplyWorkspaceEditResult>;
}

/// Default completion trigger characters; covers member access,
/// paths, attributes, and string/tag openers across the common
/// servers. (Per-server trigger characters from the initialize result
/// would require re-registration — see module note.)
const COMPLETION_TRIGGERS = [".", ":", ">", '"', "'", "/", "@", "<"];

function positionParams(model: any, position: any) {
  return {
    textDocument: { uri: model.uri.toString() },
    position: {
      line: position.lineNumber - 1,
      character: position.column - 1,
    },
  };
}

export function registerLspProviders(monaco: any, deps: LspProviderDeps): void {
  // Finisher for LSP code actions: Monaco runs CodeAction `command`s
  // through its own command service, so the action's tail work — edits
  // that touch non-open files, plus the optional LSP command — is
  // wrapped behind this id. Runs when the user picks the action.
  monaco.editor.registerCommand(
    "oxplow.lsp.applyCodeAction",
    (
      _accessor: unknown,
      languageId: string,
      normalized: NormalizedWorkspaceEdit | null,
      command: { command: string; arguments?: unknown[] } | null,
    ) => {
      void (async () => {
        if (normalized) {
          reportApplyResult(deps, "code action", await deps.applyEdits(normalized));
        }
        if (command?.command) {
          await deps
            .getClient(languageId)
            .request("workspace/executeCommand", {
              command: command.command,
              arguments: command.arguments ?? [],
            })
            .catch(() => {
              /* status banner already updated by LspClient */
            });
        }
      })();
    },
  );

  for (const languageId of allKnownLanguageIds()) {
    const active = (model: any): boolean =>
      deps.hasServer(languageId) && model.getLanguageId() === languageId;

    monaco.languages.registerDefinitionProvider(languageId, {
      provideDefinition: async (model: any, position: any) => {
        if (!active(model)) return null;
        deps.flushDoc(model);
        const result = await deps
          .getClient(languageId)
          .request<unknown>("textDocument/definition", positionParams(model, position));
        return definitionResultToMonacoLocations(monaco, result);
      },
    });

    monaco.languages.registerHoverProvider(languageId, {
      provideHover: async (model: any, position: any) => {
        if (!active(model)) return null;
        deps.flushDoc(model);
        const result = await deps
          .getClient(languageId)
          .request<any>("textDocument/hover", positionParams(model, position));
        if (!result?.contents) return null;
        return {
          contents: normalizeHoverContents(result.contents),
          range: result.range ? toMonacoRange(monaco, result.range) : undefined,
        };
      },
    });

    monaco.languages.registerReferenceProvider(languageId, {
      provideReferences: async (model: any, position: any) => {
        if (!active(model)) return [];
        deps.flushDoc(model);
        const result = await deps.getClient(languageId).request<unknown[]>(
          "textDocument/references",
          {
            ...positionParams(model, position),
            context: { includeDeclaration: true },
          },
        );
        return Array.isArray(result)
          ? result.map((item) => referenceToMonacoLocation(monaco, item)).filter(Boolean)
          : [];
      },
    });

    monaco.languages.registerCompletionItemProvider(languageId, {
      triggerCharacters: COMPLETION_TRIGGERS,
      provideCompletionItems: async (model: any, position: any) => {
        if (!active(model)) return { suggestions: [] };
        deps.flushDoc(model);
        const result = await deps
          .getClient(languageId)
          .request<unknown>("textDocument/completion", positionParams(model, position));
        const word = model.getWordUntilPosition(position);
        const defaultRange = new monaco.Range(
          position.lineNumber,
          word.startColumn,
          position.lineNumber,
          word.endColumn,
        );
        const { suggestions, incomplete } = completionResultToMonacoList(
          monaco,
          result,
          defaultRange,
        );
        return { suggestions, incomplete };
      },
    });

    monaco.languages.registerRenameProvider(languageId, {
      provideRenameEdits: async (model: any, position: any, newName: string) => {
        if (!active(model)) return { edits: [], rejectReason: "no language server" };
        deps.flushDoc(model);
        const result = await deps.getClient(languageId).request<unknown>(
          "textDocument/rename",
          { ...positionParams(model, position), newName },
        );
        const normalized = normalizeWorkspaceEdit(result);
        reportSkippedOps(deps, normalized);
        if (!normalized.files.length) {
          return { edits: [], rejectReason: "nothing to rename here" };
        }
        // Monaco's rename machinery applies the returned edit, but only
        // to open models — files without a model are written via the
        // workspace file IPC here (provideRenameEdits runs on accept,
        // so the side effect is user-initiated).
        const { open, closed } = partitionByOpenModel(
          { findModel: (uri) => monaco.editor.getModel(monaco.Uri.parse(uri)) },
          normalized.files,
        );
        if (closed.length) {
          void deps
            .applyEdits({ files: closed, skippedFileOps: 0 })
            .then((applied) => reportApplyResult(deps, "rename", applied));
        }
        return toMonacoWorkspaceEdit(monaco, { files: open, skippedFileOps: 0 });
      },
    });

    monaco.languages.registerCodeActionProvider(languageId, {
      provideCodeActions: async (model: any, range: any, context: any) => {
        if (!active(model)) return { actions: [], dispose() {} };
        deps.flushDoc(model);
        const result = await deps.getClient(languageId).request<unknown>(
          "textDocument/codeAction",
          {
            textDocument: { uri: model.uri.toString() },
            range: {
              start: { line: range.startLineNumber - 1, character: range.startColumn - 1 },
              end: { line: range.endLineNumber - 1, character: range.endColumn - 1 },
            },
            context: {
              diagnostics: markersToLspDiagnostics(context?.markers ?? []),
              only: context?.only ? [context.only] : undefined,
            },
          },
        );
        const actions = (Array.isArray(result) ? result : []).flatMap((raw) => {
          if (!raw || typeof raw !== "object") return [];
          const a = raw as {
            title?: string;
            kind?: string;
            isPreferred?: boolean;
            edit?: unknown;
            command?: { command?: string; title?: string; arguments?: unknown[] };
          };
          // Bare Command (no title+edit shape) → still offer it.
          if (typeof a.title !== "string") return [];
          const normalized = a.edit ? normalizeWorkspaceEdit(a.edit) : null;
          if (normalized) reportSkippedOps(deps, normalized);
          // Edits confined to open models ride Monaco's native code-
          // action application; anything touching a non-open file is
          // deferred to the applyCodeAction command (runs on selection,
          // partitions again at that point).
          const touchesClosedFiles =
            !!normalized?.files.length &&
            partitionByOpenModel(
              { findModel: (uri) => monaco.editor.getModel(monaco.Uri.parse(uri)) },
              normalized.files,
            ).closed.length > 0;
          const commandEdit = touchesClosedFiles ? normalized : null;
          return [
            {
              title: a.title,
              kind: a.kind,
              isPreferred: a.isPreferred,
              edit:
                normalized?.files.length && !touchesClosedFiles
                  ? toMonacoWorkspaceEdit(monaco, normalized)
                  : undefined,
              command:
                commandEdit || a.command?.command
                  ? {
                      id: "oxplow.lsp.applyCodeAction",
                      title: a.command?.title ?? a.title,
                      arguments: [languageId, commandEdit, a.command ?? null],
                    }
                  : undefined,
            },
          ];
        });
        return { actions, dispose() {} };
      },
    });

    monaco.languages.registerDocumentSymbolProvider(languageId, {
      provideDocumentSymbols: async (model: any) => {
        if (!active(model)) return [];
        deps.flushDoc(model);
        const result = await deps.getClient(languageId).request<unknown>(
          "textDocument/documentSymbol",
          { textDocument: { uri: model.uri.toString() } },
        );
        return documentSymbolsToMonaco(monaco, result);
      },
    });
  }
}

function reportSkippedOps(deps: LspProviderDeps, normalized: NormalizedWorkspaceEdit): void {
  if (normalized.skippedFileOps > 0) {
    deps.setStatus(
      `LSP: skipped ${normalized.skippedFileOps} file create/rename/delete operation(s) — apply those manually`,
    );
  }
}

function reportApplyResult(
  deps: LspProviderDeps,
  what: string,
  result: ApplyWorkspaceEditResult,
): void {
  if (result.failures.length) {
    deps.setStatus(`LSP ${what}: ${result.failures.length} file(s) failed — ${result.failures[0]}`);
  } else if (result.appliedFiles > 0) {
    deps.setStatus(`LSP ${what}: updated ${result.appliedFiles} file(s)`);
  }
}
