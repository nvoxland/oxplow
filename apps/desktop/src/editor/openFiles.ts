/// The window's open files: the one buffer store (`buffers.ts`) wired to
/// the backend and Monaco — reads through `read_at` at the working
/// revision, saves through `oxplow.file.save`, a Monaco model per buffer
/// at the file's URI, a stream's LSP mirror (`lspMirror.ts`), and the
/// worktree followed (a change on disk, a rename, a delete).
import { readAt, runCommand, runCommandForCall, statFile, subscribeWorkspaceEvents, type Stream } from "../api.js";
import { languageForPath } from "../editor-language.js";
import { registerLspProviders } from "../components/editor-lsp-providers.js";
import { LspClient, registerLspApplyEditHandler, streamFileUri } from "../lsp.js";
import { normalizeWorkspaceEdit } from "../lsp-monaco-mapping.js";
import { hasLspServer, refreshLspServers, subscribeLspServers } from "../lsp-servers-store.js";
import { applyNormalizedWorkspaceEdit, workspaceEditIOForStream } from "../lsp-workspace-edit.js";
import { unshownReason } from "../filesystem.js";
import { WORKING } from "../revision.js";
import { watchFileEvents } from "../watchFileEvents.js";
import { createBufferStore, type ModelHandle } from "./buffers.js";
import { LspMirror } from "./lspMirror.js";

/* eslint-disable @typescript-eslint/no-explicit-any */

let monaco: any = null;
/** Monaco, loaded once (its chunk is lazy), with the LSP providers
 *  registered on it once for every file of every stream. */
export async function loadMonaco(): Promise<any> {
  if (!monaco) {
    monaco = await import("monaco-editor");
    registerLspProviders(monaco, providerDeps(monaco));
  }
  return monaco;
}

/** The open file a model holds, by its URI. */
const filesByUri = new Map<string, { stream: Stream; path: string }>();

function mirrorFor(uri: string): { mirror: LspMirror; stream: Stream; path: string } {
  const file = filesByUri.get(uri);
  const mirror = file ? LspMirror.of(file.stream.id) : undefined;
  if (!file || !mirror) throw new Error("this file isn't open in a stream");
  return { mirror, ...file };
}

function providerDeps(m: any) {
  return {
    hasServer: hasLspServer,
    getClient: (uri: string, languageId: string) => mirrorFor(uri).mirror.client(languageId),
    flushDoc: (model: any) => {
      const file = filesByUri.get(model.uri.toString());
      if (file) LspMirror.of(file.stream.id)?.flush(file.path);
    },
    setStatus: (uri: string, message: string | null) => {
      const file = filesByUri.get(uri);
      if (file) LspMirror.of(file.stream.id)?.report(message);
    },
    applyEdits: (uri: string, normalized: Parameters<typeof applyNormalizedWorkspaceEdit>[2]) =>
      applyNormalizedWorkspaceEdit(m, workspaceEditIOForStream(m, mirrorFor(uri).stream), normalized),
  };
}

/** Whose markers the LSP diagnostics are (cleared with the document). */
const MARKER_OWNER = "oxplow-lsp";

function severity(m: any, s?: number): number {
  switch (s) {
    case 1:
      return m.MarkerSeverity.Error;
    case 2:
      return m.MarkerSeverity.Warning;
    case 3:
      return m.MarkerSeverity.Info;
    default:
      return m.MarkerSeverity.Hint;
  }
}

interface Diagnostic {
  severity?: number;
  message: string;
  source?: string;
  range: { start: { line: number; character: number }; end: { line: number; character: number } };
}

function setMarkers(uri: string, diagnostics: unknown[] | null): void {
  if (!monaco) return;
  const model = monaco.editor.getModel(monaco.Uri.parse(uri));
  if (!model) return;
  monaco.editor.setModelMarkers(
    model,
    MARKER_OWNER,
    (diagnostics ?? []).map((raw) => {
      const d = raw as Diagnostic;
      return {
        severity: severity(monaco, d.severity),
        message: d.message,
        source: d.source,
        startLineNumber: d.range.start.line + 1,
        startColumn: d.range.start.character + 1,
        endLineNumber: d.range.end.line + 1,
        endColumn: d.range.end.character + 1,
      };
    }),
  );
}

/** Answer the server's `workspace/applyEdit` for `stream`: open files
 *  through their models (the draft and its undo), the rest written. */
function answerApplyEdit(stream: Stream, status: (message: string | null) => void): () => void {
  return registerLspApplyEditHandler(async (request) => {
    if (request.streamId !== stream.id || !monaco) return null;
    const normalized = normalizeWorkspaceEdit(request.edit);
    const result = await applyNormalizedWorkspaceEdit(monaco, workspaceEditIOForStream(monaco, stream), normalized);
    const label = request.label ? ` (${request.label})` : "";
    if (result.failures.length || normalized.skippedFileOps > 0) {
      status(
        `LSP edit${label}: ${result.appliedFiles} file(s) applied, ` +
          `${result.failures.length} failed, ${normalized.skippedFileOps} file op(s) skipped`,
      );
      return { applied: false, failureReason: result.failures[0] ?? "file create/rename/delete operations are not supported" };
    }
    if (result.appliedFiles > 0) status(`LSP edit${label}: updated ${result.appliedFiles} file(s)`);
    return { applied: true };
  });
}

/** Follow `streamId`'s worktree: a file changed on disk (debounced per
 *  path), renamed or deleted. */
function watch(
  streamId: string,
  on: {
    changed(path: string): void;
    everythingChanged(): void;
    renamed(from: string, to: string): void;
    deleted(path: string): void;
  },
): () => void {
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  const offDisk = subscribeWorkspaceEvents(streamId, (event) => {
    if (event.overflow) on.everythingChanged();
    for (const path of event.paths) {
      const t = timers.get(path);
      if (t) clearTimeout(t);
      timers.set(
        path,
        setTimeout(() => {
          timers.delete(path);
          on.changed(path);
        }, 75),
      );
    }
  });
  const offLog = watchFileEvents(streamId, (event) => {
    if (event.kind === "renamed") on.renamed(event.from, event.to);
    else on.deleted(event.path);
  });
  return () => {
    offDisk();
    offLog();
    for (const t of timers.values()) clearTimeout(t);
  };
}

export const buffers = createBufferStore({
  // `stat` first: a binary or too-large file is unshown, never read.
  read: async (stream, path) => {
    const stat = await statFile(stream, path, { rev: WORKING });
    if (!stat) return null;
    const unshown = unshownReason(stat);
    if (unshown) return { unshown };
    const text = await readAt(stream, path, { rev: WORKING });
    return text === null ? null : { text };
  },
  save: async (stream, path, content, call) => {
    // An audited write that logs `file.saved` (its content isn't kept in
    // the record); an agent's Save is written as the agent.
    const input = { stream, path, content };
    await (call ? runCommandForCall(call, "oxplow.file.save", input) : runCommand("oxplow.file.save", input));
  },
  model: async (stream, path, text): Promise<ModelHandle> => {
    const m = await loadMonaco();
    const key = streamFileUri(stream, path);
    const uri = m.Uri.parse(key);
    m.editor.getModel(uri)?.dispose();
    const model = m.editor.createModel(text, languageForPath(path), uri);
    filesByUri.set(key, { stream, path });
    model.onWillDispose(() => {
      if (filesByUri.get(key)?.path === path) filesByUri.delete(key);
    });
    return model;
  },
  services: (stream, store) => {
    // Another worktree may carry another `.oxplow/project.yaml`.
    void refreshLspServers();
    return new LspMirror(stream, store, {
      hasServer: hasLspServer,
      onServersChanged: subscribeLspServers,
      client: (s, lang) => new LspClient(s, lang),
      markers: setMarkers,
      uri: streamFileUri,
      applyEdit: answerApplyEdit,
    });
  },
  watch,
});
