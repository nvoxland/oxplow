//! LSP client lifecycle for `EditorPane`, extracted into a hook.
//!
//! Owns the per-language `LspClient` cache and everything that hangs off
//! it: diagnostics → Monaco markers, the `didOpen`/`didChange`/`didClose`/
//! `didSave` document sync as files open/edit/close/save, the LSP status
//! banner state (+ the "Install <pkg>" suggestion/flow), and disposal on
//! both stream-switch and unmount. `EditorPane` keeps the Monaco editor
//! itself and calls `ensureLspClient` from its provider registration.
//!
//! didChange rides `DocumentSyncTracker` (per-path monotonic versions +
//! debounce); `flushPendingChanges` must be called before didSave and
//! before any positional request so the server never sees stale text.

import { useCallback, useEffect, useRef, useState, type MutableRefObject } from "react";

import { installLspPackage, type Stream } from "../api.js";
import type { OpenFileState } from "../editor-session.js";
import { languageForPath } from "../editor-language.js";
import { hasLspServer, refreshLspServers } from "../lsp-servers-store.js";
import { DocumentSyncTracker } from "../lsp-document-sync.js";
import { normalizeWorkspaceEdit } from "../lsp-monaco-mapping.js";
import {
  applyNormalizedWorkspaceEdit,
  workspaceEditIOForStream,
} from "../lsp-workspace-edit.js";
import { LspClient, registerLspApplyEditHandler, streamFileUri } from "../lsp.js";
import { getSuggestedLspPackage } from "../lspSuggestions.js";
import { logUi } from "../logger.js";

/* eslint-disable @typescript-eslint/no-explicit-any */

function diagnosticSeverity(monaco: any, severity?: number): number {
  switch (severity) {
    case 1:
      return monaco.MarkerSeverity.Error;
    case 2:
      return monaco.MarkerSeverity.Warning;
    case 3:
      return monaco.MarkerSeverity.Info;
    default:
      return monaco.MarkerSeverity.Hint;
  }
}

export interface LspInstallSuggestion {
  language: string;
  pkg: string;
}

export interface LspClientsApi {
  /// Get-or-create the cached client for a language; registers its
  /// diagnostics + status handlers on first creation.
  ensureLspClient: (currentStream: Stream, languageId: string) => LspClient;
  /// Send any pending (debounced) didChange for `path` right now. Call
  /// before issuing a positional request against that document.
  flushPendingChanges: (path: string | null) => void;
  lspInstallSuggestion: LspInstallSuggestion | null;
  lspInstalling: boolean;
  /// Run the suggested Mason install, then drop the stale client so the
  /// next open retries with the new binary.
  installSuggested: () => Promise<void>;
}

/// `setLspStatus` is passed in (not owned) because the status banner is
/// shared — EditorPane's blame + go-to-definition also write to it.
export function useLspClients(opts: {
  monacoRef: MutableRefObject<any>;
  filePathRef: MutableRefObject<string | null>;
  stream: Stream;
  openFiles: Record<string, OpenFileState>;
  openFileOrder: string[];
  setLspStatus: (status: string | null) => void;
}): LspClientsApi {
  const { monacoRef, filePathRef, stream, openFiles, openFileOrder, setLspStatus } = opts;

  const lspClientsRef = useRef(new Map<string, LspClient>());
  const trackedOpenDocsRef = useRef(new Map<string, string>());
  const trackedSavedContentRef = useRef(new Map<string, string>());
  const diagnosticsDisposersRef = useRef<(() => void)[]>([]);
  const markerOwnerRef = useRef(`oxplow-lsp-${stream.id}`);
  const streamRef = useRef(stream);
  streamRef.current = stream;
  const [lspInstallSuggestion, setLspInstallSuggestion] = useState<LspInstallSuggestion | null>(null);
  const [lspInstalling, setLspInstalling] = useState(false);

  const ensureLspClient = useCallback(
    (currentStream: Stream, languageId: string): LspClient => {
      let client = lspClientsRef.current.get(languageId);
      if (!client) {
        client = new LspClient(currentStream.id, languageId);
        diagnosticsDisposersRef.current.push(
          client.onDiagnostics((uri, diagnostics) => {
            const monaco = monacoRef.current;
            if (!monaco) return;
            const model = monaco.editor.getModel(monaco.Uri.parse(uri));
            if (!model) return;
            monaco.editor.setModelMarkers(
              model,
              markerOwnerRef.current,
              diagnostics.map((diagnostic) => ({
                severity: diagnosticSeverity(monaco, diagnostic.severity),
                message: diagnostic.message,
                source: diagnostic.source,
                startLineNumber: diagnostic.range.start.line + 1,
                startColumn: diagnostic.range.start.character + 1,
                endLineNumber: diagnostic.range.end.line + 1,
                endColumn: diagnostic.range.end.character + 1,
              })),
            );
          }),
        );
        diagnosticsDisposersRef.current.push(
          client.onStatus((message) => {
            const currentLanguage = languageForPath(filePathRef.current);
            if (currentLanguage !== languageId) return;
            setLspStatus(message);
            if (message && /LSP unavailable/i.test(message)) {
              const pkg = getSuggestedLspPackage(languageId);
              setLspInstallSuggestion(pkg ? { language: languageId, pkg } : null);
            } else if (!message) {
              setLspInstallSuggestion(null);
            }
          }),
        );
        lspClientsRef.current.set(languageId, client);
      }
      return client;
    },
    [monacoRef, filePathRef, setLspStatus],
  );

  // didChange sender: per-path monotonic version + debounce. Lives for
  // the hook's lifetime; reset on stream switch.
  const syncTrackerRef = useRef<DocumentSyncTracker | null>(null);
  if (!syncTrackerRef.current) {
    syncTrackerRef.current = new DocumentSyncTracker((path, text, version) => {
      const currentStream = streamRef.current;
      const languageId = trackedOpenDocsRef.current.get(path) ?? languageForPath(path);
      ensureLspClient(currentStream, languageId).notify("textDocument/didChange", {
        textDocument: { uri: streamFileUri(currentStream, path), version },
        contentChanges: [{ text }],
      });
    });
  }
  const syncTracker = syncTrackerRef.current;

  const flushPendingChanges = useCallback(
    (path: string | null) => {
      if (path) syncTracker.flush(path);
    },
    [syncTracker],
  );

  // Sync didOpen/didClose as files open/close. Mirrors the editor's open
  // set into the LSP server; closing a doc clears its markers.
  useEffect(() => {
    const monaco = monacoRef.current;
    if (!monaco) return;
    const t0 = performance.now();
    logUi("debug", "editor: lsp-sync start", { fileCount: openFileOrder.length });
    const nextOpenDocs = new Map<string, string>();
    for (const path of openFileOrder) {
      const openFile = openFiles[path];
      if (!openFile || openFile.isLoading) continue;
      const languageId = languageForPath(path);
      if (!hasLspServer(languageId)) continue;
      const uri = streamFileUri(stream, path);
      nextOpenDocs.set(path, languageId);
      if (!trackedOpenDocsRef.current.has(path)) {
        const version = syncTracker.open(path, openFile.draftContent);
        ensureLspClient(stream, languageId).notify("textDocument/didOpen", {
          textDocument: {
            uri,
            languageId,
            version,
            text: openFile.draftContent,
          },
        });
      }
      trackedSavedContentRef.current.set(path, openFile.savedContent);
    }

    for (const [path, languageId] of trackedOpenDocsRef.current) {
      if (nextOpenDocs.has(path)) continue;
      syncTracker.close(path);
      ensureLspClient(stream, languageId).notify("textDocument/didClose", {
        textDocument: { uri: streamFileUri(stream, path) },
      });
      const model = monaco.editor.getModel(monaco.Uri.parse(streamFileUri(stream, path)));
      if (model) {
        monaco.editor.setModelMarkers(model, markerOwnerRef.current, []);
      }
      trackedSavedContentRef.current.delete(path);
    }

    trackedOpenDocsRef.current = nextOpenDocs;
    logUi("debug", "editor: lsp-sync end", {
      fileCount: openFileOrder.length,
      tracked: trackedOpenDocsRef.current.size,
      ms: Math.round(performance.now() - t0),
    });
  }, [openFileOrder, openFiles, stream, ensureLspClient, monacoRef, syncTracker]);

  // Feed draft edits into the didChange tracker (debounced full-text
  // sync), and notify didSave when a tracked doc's saved content
  // advances to match its draft (i.e. an actual save).
  useEffect(() => {
    for (const [path, languageId] of trackedOpenDocsRef.current) {
      const openFile = openFiles[path];
      if (!openFile) continue;
      syncTracker.changed(path, openFile.draftContent);
      const previousSaved = trackedSavedContentRef.current.get(path);
      if (previousSaved === undefined || previousSaved === openFile.savedContent) continue;
      trackedSavedContentRef.current.set(path, openFile.savedContent);
      if (openFile.savedContent !== openFile.draftContent) continue;
      syncTracker.flush(path);
      ensureLspClient(stream, languageId).notify("textDocument/didSave", {
        textDocument: { uri: streamFileUri(stream, path) },
        text: openFile.savedContent,
      });
    }
  }, [openFiles, stream, ensureLspClient, syncTracker]);

  // On stream switch, drop every client + tracked-doc state and reset the
  // marker owner so the new stream's diagnostics don't collide. Also
  // refresh the server list — a different worktree may carry a different
  // oxplow.yaml.
  useEffect(() => {
    markerOwnerRef.current = `oxplow-lsp-${stream.id}`;
    setLspStatus(null);
    void refreshLspServers();
    syncTracker.reset();
    for (const client of lspClientsRef.current.values()) {
      client.dispose();
    }
    lspClientsRef.current.clear();
    trackedOpenDocsRef.current.clear();
    trackedSavedContentRef.current.clear();
    diagnosticsDisposersRef.current.forEach((dispose) => dispose());
    diagnosticsDisposersRef.current = [];
  }, [stream.id, setLspStatus, syncTracker]);

  // Server-initiated workspace/applyEdit: apply across open models and
  // non-open files, then answer the server honestly. Declines (returns
  // null) for other streams so their own editor can claim the request.
  useEffect(() => {
    return registerLspApplyEditHandler(async (request) => {
      if (request.streamId !== streamRef.current.id) return null;
      const monaco = monacoRef.current;
      if (!monaco) return null;
      const normalized = normalizeWorkspaceEdit(request.edit);
      const result = await applyNormalizedWorkspaceEdit(
        monaco,
        workspaceEditIOForStream(monaco, streamRef.current),
        normalized,
      );
      const label = request.label ? ` (${request.label})` : "";
      if (result.failures.length || normalized.skippedFileOps > 0) {
        setLspStatus(
          `LSP edit${label}: ${result.appliedFiles} file(s) applied, ` +
            `${result.failures.length} failed, ${normalized.skippedFileOps} file op(s) skipped`,
        );
        return {
          applied: false,
          failureReason:
            result.failures[0] ?? "file create/rename/delete operations are not supported",
        };
      }
      if (result.appliedFiles > 0) {
        setLspStatus(`LSP edit${label}: updated ${result.appliedFiles} file(s)`);
      }
      return { applied: true };
    });
  }, [monacoRef, setLspStatus]);

  // Dispose everything on unmount (the editor's setup effect used to own
  // this half of the cleanup).
  useEffect(() => {
    const clients = lspClientsRef.current;
    const disposers = diagnosticsDisposersRef.current;
    const tracker = syncTracker;
    return () => {
      tracker.reset();
      disposers.forEach((dispose) => dispose());
      for (const client of clients.values()) {
        client.dispose();
      }
      clients.clear();
    };
  }, [syncTracker]);

  const installSuggested = useCallback(async () => {
    const suggestion = lspInstallSuggestion;
    if (!suggestion) return;
    const { pkg, language: lang } = suggestion;
    setLspInstalling(true);
    try {
      await installLspPackage(pkg);
      setLspStatus(`Installed ${pkg} — reopen file to start the server`);
      setLspInstallSuggestion(null);
      await refreshLspServers();
      // Drop the cached client so the next open re-tries.
      const stale = lspClientsRef.current.get(lang);
      if (stale) {
        stale.dispose();
        lspClientsRef.current.delete(lang);
      }
    } catch (err) {
      setLspStatus(`Install failed: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      setLspInstalling(false);
    }
  }, [lspInstallSuggestion, setLspStatus]);

  return { ensureLspClient, flushPendingChanges, lspInstallSuggestion, lspInstalling, installSuggested };
}
