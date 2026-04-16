import type { Stream } from "./api.js";

export interface EditorNavigationTarget {
  path: string;
  line: number;
  column: number;
}

interface JsonRpcRequest {
  jsonrpc: "2.0";
  id: number;
  method: string;
  params?: unknown;
}

interface JsonRpcNotification {
  jsonrpc: "2.0";
  method: string;
  params?: unknown;
}

interface JsonRpcSuccess {
  jsonrpc: "2.0";
  id: number;
  result?: unknown;
}

interface JsonRpcFailure {
  jsonrpc: "2.0";
  id: number;
  error: { code: number; message: string; data?: unknown };
}

type JsonRpcMessage = JsonRpcNotification | JsonRpcSuccess | JsonRpcFailure;

export interface LspDiagnostic {
  range: {
    start: { line: number; character: number };
    end: { line: number; character: number };
  };
  severity?: number;
  message: string;
  source?: string;
}

type DiagnosticsListener = (uri: string, diagnostics: LspDiagnostic[]) => void;
type StatusListener = (message: string | null) => void;

export class LspClient {
  private clientId: string | null = null;
  private nextId = 1;
  private openPromise: Promise<void> | null = null;
  private pending = new Map<number, { resolve(value: unknown): void; reject(reason?: unknown): void }>();
  private queued: string[] = [];
  private diagnosticsListeners = new Set<DiagnosticsListener>();
  private statusListeners = new Set<StatusListener>();
  private unsubscribeMessages: (() => void) | null = null;

  constructor(
    private readonly streamId: string,
    private readonly languageId: string,
  ) {}

  onDiagnostics(listener: DiagnosticsListener): () => void {
    this.diagnosticsListeners.add(listener);
    return () => this.diagnosticsListeners.delete(listener);
  }

  onStatus(listener: StatusListener): () => void {
    this.statusListeners.add(listener);
    return () => this.statusListeners.delete(listener);
  }

  async request<T>(method: string, params?: unknown): Promise<T> {
    await this.ensureOpen();
    const id = this.nextId++;
    const payload: JsonRpcRequest = { jsonrpc: "2.0", id, method, params };
    const promise = new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (value: unknown) => void, reject });
    });
    this.send(JSON.stringify(payload));
    return promise;
  }

  notify(method: string, params?: unknown): void {
    const payload: JsonRpcNotification = { jsonrpc: "2.0", method, params };
    void this.ensureOpen().then(() => {
      this.send(JSON.stringify(payload));
    }).catch((error) => {
      this.emitStatus(`LSP unavailable: ${errorMessage(error)}`);
    });
  }

  dispose(): void {
    this.openPromise = null;
    const clientId = this.clientId;
    this.clientId = null;
    for (const pending of this.pending.values()) {
      pending.reject(new Error("lsp client disposed"));
    }
    this.pending.clear();
    this.queued = [];
    this.unsubscribeMessages?.();
    this.unsubscribeMessages = null;
    if (clientId) {
      void window.newdeApi.closeLspClient(clientId);
    }
  }

  private ensureOpen(): Promise<void> {
    if (this.clientId) return Promise.resolve();
    if (this.openPromise) return this.openPromise;
    this.openPromise = new Promise<void>((resolve, reject) => {
      let opened = false;
      this.unsubscribeMessages = window.newdeApi.onLspEvent((event) => {
        if (event.clientId !== this.clientId) return;
        const parsed = parseJsonRpc(event.message);
        if (!parsed) return;
        this.handleMessage(parsed);
      });
      void window.newdeApi.openLspClient(this.streamId, this.languageId).then((clientId) => {
        this.clientId = clientId;
        opened = true;
        this.emitStatus(null);
        for (const message of this.queued) this.send(message);
        this.queued = [];
        resolve();
      }).catch((error) => {
        this.emitStatus(`LSP unavailable for ${this.languageId}`);
        this.openPromise = null;
        this.clientId = null;
        this.unsubscribeMessages?.();
        this.unsubscribeMessages = null;
        if (!opened) reject(new Error(`failed to open LSP for ${this.languageId}: ${errorMessage(error)}`));
        for (const pending of this.pending.values()) {
          pending.reject(new Error("lsp connection closed"));
        }
        this.pending.clear();
      });
    });
    return this.openPromise.catch((error) => {
      this.openPromise = null;
      this.clientId = null;
      throw error;
    });
  }

  private send(payload: string): void {
    if (!this.clientId) {
      this.queued.push(payload);
      return;
    }
    void window.newdeApi.sendLspMessage(this.clientId, payload).catch((error) => {
      this.emitStatus(`LSP unavailable: ${errorMessage(error)}`);
    });
  }

  private handleMessage(message: JsonRpcMessage): void {
    if ("method" in message) {
      if (message.method === "textDocument/publishDiagnostics") {
        const params = message.params as { uri?: string; diagnostics?: LspDiagnostic[] } | undefined;
        if (!params?.uri || !Array.isArray(params.diagnostics)) return;
        for (const listener of this.diagnosticsListeners) {
          listener(params.uri, params.diagnostics);
        }
      } else if (message.method === "$/newde/status") {
        const params = message.params as { message?: string | null } | undefined;
        this.emitStatus(params?.message ?? null);
      }
      return;
    }

    const pending = this.pending.get(message.id);
    if (!pending) return;
    this.pending.delete(message.id);
    if ("error" in message) {
      pending.reject(new Error(message.error.message));
      this.emitStatus(`LSP error: ${message.error.message}`);
      return;
    }
    pending.resolve(message.result);
  }

  private emitStatus(message: string | null): void {
    for (const listener of this.statusListeners) {
      listener(message);
    }
  }
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

function parseJsonRpc(raw: unknown): JsonRpcMessage | null {
  if (typeof raw !== "string") return null;
  try {
    return JSON.parse(raw) as JsonRpcMessage;
  } catch {
    return null;
  }
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
