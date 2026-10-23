import { desktopBridge, lspNotify, lspRequest, respondLspApplyEdit, type Stream } from "./api.js";

export interface EditorNavigationTarget {
  path: string;
  line: number;
  column: number;
}

export interface LspDiagnostic {
  range: {
    start: { line: number; character: number };
    end: { line: number; character: number };
  };
  severity?: number;
  message: string;
  source?: string;
}

/// Wire shape of `LspSessionEvent` (crates/oxplow-app/src/lsp_sessions.rs),
/// delivered on the `lsp:event` channel. Defined manually because the
/// event type isn't referenced by any command signature, so specta
/// doesn't export it into bindings.ts.
export type LspSessionEventPayload =
  | {
      kind: "serverNotification";
      streamId: string;
      language: string;
      method: string;
      params: unknown;
    }
  | {
      kind: "sessionStatus";
      streamId: string;
      language: string;
      status: "ready" | "crashed" | "restarted" | "stopped";
      message: string | null;
    }
  | {
      kind: "applyEditRequest";
      streamId: string;
      language: string;
      token: number;
      label: string | null;
      edit: unknown;
    };

type DiagnosticsListener = (uri: string, diagnostics: LspDiagnostic[]) => void;
type StatusListener = (message: string | null) => void;

/// One `lsp:event` subscription for the whole renderer, demuxed to the
/// live clients by (streamId, languageId). Started lazily when the
/// first client is constructed.
const clientRegistry = new Map<string, Set<LspClient>>();
let eventUnsubscribe: (() => void) | null = null;

function registryKey(streamId: string, languageId: string): string {
  return `${streamId}\u0000${languageId}`;
}

function ensureEventSubscription(): void {
  if (eventUnsubscribe) return;
  try {
    eventUnsubscribe = desktopBridge().onLspEvent((event) =>
      handleLspSessionEvent(event as LspSessionEventPayload),
    );
  } catch {
    // No transport (tests). Events can still be injected via
    // handleLspSessionEvent directly.
  }
}

/// Exported for tests — production traffic arrives via the bridge
/// subscription above.
export function handleLspSessionEvent(event: LspSessionEventPayload): void {
  if (!event || typeof event !== "object" || !("kind" in event)) return;
  if (event.kind === "applyEditRequest") {
    void dispatchApplyEdit(event);
    return;
  }
  const clients = clientRegistry.get(registryKey(event.streamId, event.language));
  if (!clients?.size) return;
  for (const client of clients) {
    client.handleSessionEvent(event);
  }
}

/// A server-initiated workspace/applyEdit, offered to each registered
/// handler in turn. Return null to decline (e.g. not your stream); the
/// first non-null verdict is sent back to the server.
export type LspApplyEditHandler = (request: {
  streamId: string;
  language: string;
  label: string | null;
  edit: unknown;
}) => Promise<{ applied: boolean; failureReason?: string } | null>;

const applyEditHandlers = new Set<LspApplyEditHandler>();

export function registerLspApplyEditHandler(handler: LspApplyEditHandler): () => void {
  applyEditHandlers.add(handler);
  return () => applyEditHandlers.delete(handler);
}

type ApplyEditResponder = (
  token: number,
  applied: boolean,
  failureReason?: string,
) => Promise<void>;

let applyEditResponder: ApplyEditResponder = respondLspApplyEdit;

/// Test seam — production responses go through the RPC.
export function _setApplyEditResponderForTests(responder: ApplyEditResponder | null): void {
  applyEditResponder = responder ?? respondLspApplyEdit;
}

async function dispatchApplyEdit(
  event: Extract<LspSessionEventPayload, { kind: "applyEditRequest" }>,
): Promise<void> {
  let verdict: { applied: boolean; failureReason?: string } | null = null;
  for (const handler of applyEditHandlers) {
    try {
      verdict = await handler(event);
    } catch (error) {
      verdict = { applied: false, failureReason: errorMessage(error) };
    }
    if (verdict) break;
  }
  verdict ??= { applied: false, failureReason: "no editor attached for this stream" };
  await applyEditResponder(event.token, verdict.applied, verdict.failureReason).catch(() => {
    // Backend gone or token already timed out — nothing to surface.
  });
}

/// Thin facade over the shared backend LSP session for one
/// (stream, language). Requests/notifications ride the `lsp_request` /
/// `lsp_notify` RPCs — the backend owns the server process, the
/// initialize handshake, and crash/restart recovery; this class only
/// adds listener fan-out and friendly status messages.
export class LspClient {
  private diagnosticsListeners = new Set<DiagnosticsListener>();
  private statusListeners = new Set<StatusListener>();
  private disposed = false;

  constructor(
    private readonly streamId: string,
    private readonly languageId: string,
  ) {
    const key = registryKey(streamId, languageId);
    let set = clientRegistry.get(key);
    if (!set) {
      set = new Set();
      clientRegistry.set(key, set);
    }
    set.add(this);
    ensureEventSubscription();
  }

  onDiagnostics(listener: DiagnosticsListener): () => void {
    this.diagnosticsListeners.add(listener);
    return () => this.diagnosticsListeners.delete(listener);
  }

  onStatus(listener: StatusListener): () => void {
    this.statusListeners.add(listener);
    return () => this.statusListeners.delete(listener);
  }

  async request<T>(method: string, params?: unknown): Promise<T> {
    try {
      const result = await lspRequest(this.streamId, this.languageId, method, params ?? {});
      this.emitStatus(null);
      return result as T;
    } catch (error) {
      this.emitStatus(statusForError(error));
      throw error;
    }
  }

  notify(method: string, params?: unknown): void {
    void lspNotify(this.streamId, this.languageId, method, params ?? {}).catch((error) => {
      this.emitStatus(statusForError(error));
    });
  }

  dispose(): void {
    this.disposed = true;
    this.diagnosticsListeners.clear();
    this.statusListeners.clear();
    const key = registryKey(this.streamId, this.languageId);
    const set = clientRegistry.get(key);
    set?.delete(this);
    if (set && set.size === 0) clientRegistry.delete(key);
  }

  handleSessionEvent(event: LspSessionEventPayload): void {
    if (this.disposed) return;
    if (event.kind === "serverNotification") {
      if (event.method === "textDocument/publishDiagnostics") {
        const params = event.params as
          | { uri?: string; diagnostics?: LspDiagnostic[] }
          | undefined;
        if (!params?.uri || !Array.isArray(params.diagnostics)) return;
        for (const listener of this.diagnosticsListeners) {
          listener(params.uri, params.diagnostics);
        }
      } else if (event.method === "window/showMessage") {
        const params = event.params as { message?: string } | undefined;
        if (params?.message) this.emitStatus(`LSP: ${params.message}`);
      }
      return;
    }
    if (event.kind === "sessionStatus") {
      switch (event.status) {
        case "crashed":
          this.emitStatus("LSP server crashed — it restarts on the next request");
          break;
        case "ready":
        case "restarted":
          this.emitStatus(null);
          break;
        case "stopped":
          break;
      }
    }
  }

  private emitStatus(message: string | null): void {
    for (const listener of this.statusListeners) {
      listener(message);
    }
  }
}

/// Map RPC failures to the banner text. "LSP unavailable" is load-
/// bearing: the editor keys the install-suggestion flow off it.
function statusForError(error: unknown): string {
  const message = errorMessage(error);
  if (/no language server configured/i.test(message)) {
    return `LSP unavailable: ${message}`;
  }
  return `LSP error: ${message}`;
}

export function streamFileUri(stream: Stream, filePath: string): string {
  const base = ensureLeadingSlash(normalizeSlashes(stream.worktree_path));
  const rel = normalizeSlashes(filePath).replace(/^\/+/, "");
  return encodeFileUri(`${base}/${rel}`);
}

export function relativePathFromFileUri(stream: Stream, uri: string): string | null {
  if (!uri.startsWith("file://")) return null;
  const decoded = decodeURIComponent(uri.slice("file://".length));
  const path = normalizeSlashes(decoded);
  const root = ensureLeadingSlash(normalizeSlashes(stream.worktree_path));
  if (path === root) return "";
  if (!path.startsWith(`${root}/`)) return null;
  return path.slice(root.length + 1);
}

export function toEditorNavigationTarget(stream: Stream, uri: string, range?: {
  start?: { line?: number; character?: number };
}): EditorNavigationTarget | null {
  const path = relativePathFromFileUri(stream, uri);
  if (path == null) return null;
  return {
    path,
    line: (range?.start?.line ?? 0) + 1,
    column: (range?.start?.character ?? 0) + 1,
  };
}

function encodeFileUri(path: string): string {
  return `file://${encodeURI(path)}`;
}

function ensureLeadingSlash(path: string): string {
  return path.startsWith("/") ? path : `/${path}`;
}

function normalizeSlashes(path: string): string {
  return path.replace(/\\/g, "/");
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
