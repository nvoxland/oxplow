import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import type { BridgeSocket } from "../terminal/bridge-socket.js";
import type { Logger } from "../core/logger.js";
import type { Stream } from "../persistence/stream-store.js";

interface JsonRpcRequest {
  jsonrpc: "2.0";
  id?: number | string;
  method: string;
  params?: unknown;
}

interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: number | string;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}

type JsonRpcMessage = JsonRpcRequest | JsonRpcResponse;

export interface LanguageServerRegistration {
  languageId: string;
  extensions: string[];
  command: string;
  args: string[];
}

export interface LspDiagnostic {
  range: {
    start: { line: number; character: number };
    end: { line: number; character: number };
  };
  severity?: number;
  message: string;
  source?: string;
  code?: string | number;
}

interface PendingRequest {
  ws: BridgeSocket;
  clientId: number | string;
}

const requireFromHere = createRequire(import.meta.url);
const TYPESCRIPT_LANGUAGE_SERVER_CLI = requireFromHere.resolve("typescript-language-server/lib/cli.mjs");

const BUILT_IN_REGISTRATIONS: LanguageServerRegistration[] = [
  {
    languageId: "typescript",
    extensions: [".ts", ".tsx"],
    command: process.execPath,
    args: [TYPESCRIPT_LANGUAGE_SERVER_CLI, "--stdio"],
  },
  {
    languageId: "javascript",
    extensions: [".js", ".jsx", ".mjs", ".cjs"],
    command: process.execPath,
    args: [TYPESCRIPT_LANGUAGE_SERVER_CLI, "--stdio"],
  },
];

const REGISTRY: LanguageServerRegistration[] = BUILT_IN_REGISTRATIONS.map((entry) => ({
  ...entry,
  extensions: entry.extensions.map((extension) => extension.toLowerCase()),
}));

export function registerLanguageServer(registration: LanguageServerRegistration): void {
  const normalized: LanguageServerRegistration = {
    ...registration,
    extensions: registration.extensions.map((extension) => extension.toLowerCase()),
  };
  const existing = REGISTRY.findIndex((entry) => entry.languageId === normalized.languageId);
  if (existing >= 0) {
    REGISTRY[existing] = normalized;
  } else {
    REGISTRY.push(normalized);
  }
}

export function unregisterLanguageServer(languageId: string): void {
  const index = REGISTRY.findIndex((entry) => entry.languageId === languageId);
  if (index >= 0) REGISTRY.splice(index, 1);
}

export function listRegisteredLanguageServers(): LanguageServerRegistration[] {
  return REGISTRY.map((entry) => ({ ...entry, extensions: [...entry.extensions], args: [...entry.args] }));
}

export class LspSessionManager {
  private sessions = new Map<string, LspSession>();

  constructor(private readonly logger: Logger) {}

  async attachClient(ws: BridgeSocket, stream: Stream, languageId: string): Promise<void> {
    const session = await this.ensureSession(stream, languageId);
    session.attachClient(ws);
  }

  async getSession(stream: Stream, languageId: string): Promise<LspSession> {
    return this.ensureSession(stream, languageId);
  }

  async dispose(): Promise<void> {
    await Promise.all([...this.sessions.values()].map((session) => session.dispose()));
    this.sessions.clear();
  }

  private async ensureSession(stream: Stream, languageId: string): Promise<LspSession> {
    const key = `${stream.id}:${languageId}`;
    let session = this.sessions.get(key);
    if (!session) {
      const registration = registrationForLanguage(languageId);
      if (!registration) throw new Error(`LSP not configured for language: ${languageId}`);
      session = new LspSession(stream, registration, this.logger.child({ streamId: stream.id, languageId }));
      this.sessions.set(key, session);
      try {
        await session.initialize();
      } catch (error) {
        this.sessions.delete(key);
        await session.dispose();
        throw error;
      }
    }
    return session;
  }
}

export function lspLanguageIdForPath(path: string): string | null {
  const lower = path.toLowerCase();
  const registration = REGISTRY.find((candidate) => candidate.extensions.some((extension) => lower.endsWith(extension)));
  return registration?.languageId ?? null;
}

export class LspSession {
  private proc: ChildProcessWithoutNullStreams | null = null;
  private buffer = "";
  private contentLength: number | null = null;
  private nextServerId = 1;
  private initialized = false;
  private clients = new Set<BridgeSocket>();
  private pending = new Map<number | string, PendingRequest>();
  private openDocuments = new Map<string, { version: number; text: string }>();
  private diagnosticsByUri = new Map<string, LspDiagnostic[]>();
  private diagnosticsWaiters = new Map<string, ((diagnostics: LspDiagnostic[]) => void)[]>();

  constructor(
    private readonly stream: Stream,
    private readonly registration: LanguageServerRegistration,
    private readonly logger: Logger,
  ) {}

  async initialize(): Promise<void> {
    if (this.initialized) return;
    // If the command looks like an absolute path, verify it exists up-front
    // for a friendlier error. Otherwise defer to spawn + PATH resolution.
    if ((this.registration.command.startsWith("/") || this.registration.command.match(/^[a-zA-Z]:[\\/]/)) &&
        !existsSync(this.registration.command)) {
      throw new Error(`missing language server executable: ${this.registration.command}`);
    }
    const proc = spawn(this.registration.command, this.registration.args, {
      cwd: this.stream.worktree_path,
      stdio: "pipe",
      env: { ...process.env, ELECTRON_RUN_AS_NODE: "1" },
    });
    this.proc = proc;
    proc.stdout.setEncoding("utf8");
    proc.stdout.on("data", (chunk: string) => this.readChunk(chunk));
    proc.stderr.setEncoding("utf8");
    proc.stderr.on("data", (chunk: string) => {
      this.logger.warn("language server stderr", { chunk: chunk.trim() || undefined });
    });
    proc.on("exit", (code, signal) => {
      this.logger.warn("language server exited", { code: code ?? undefined, signal: signal ?? undefined });
      this.broadcast({
        jsonrpc: "2.0",
        method: "$/oxplow/status",
        params: { message: `LSP server exited for ${this.registration.languageId}` },
      });
      this.proc = null;
      this.initialized = false;
      for (const [id, pending] of this.pending) {
        pending.ws.send(JSON.stringify({
          jsonrpc: "2.0",
          id: pending.clientId,
          error: { code: -32000, message: `language server exited (${code ?? signal ?? "unknown"})` },
        }));
        this.pending.delete(id);
      }
    });

      const result = await this.requestToServer("initialize", {
      processId: process.pid,
      rootUri: fileUri(this.stream.worktree_path),
      capabilities: {
        textDocument: {
          definition: { dynamicRegistration: false, linkSupport: true },
          hover: { dynamicRegistration: false, contentFormat: ["markdown", "plaintext"] },
          references: { dynamicRegistration: false },
          publishDiagnostics: { relatedInformation: true },
          synchronization: {
            dynamicRegistration: false,
            didSave: true,
            willSave: false,
            willSaveWaitUntil: false,
          },
        },
        workspace: {
          configuration: true,
          workspaceFolders: true,
        },
      },
      workspaceFolders: [
        {
          uri: fileUri(this.stream.worktree_path),
          name: this.stream.title,
        },
      ],
    });
    this.logger.info("initialized language server", {
      languageId: this.registration.languageId,
      capabilities: result && typeof result === "object" ? Object.keys(result as Record<string, unknown>) : [],
    });
    this.notifyServer("initialized", {});
    this.initialized = true;
  }

  attachClient(ws: BridgeSocket): void {
    this.clients.add(ws);
    ws.on("message", (raw) => {
      const message = parseJsonRpc(raw.toString());
      if (!message || !("jsonrpc" in message)) return;
      if ("method" in message) {
        if (message.id !== undefined) {
          this.forwardRequest(ws, message);
        } else {
          this.notifyServer(message.method, message.params);
        }
      }
    });
    ws.on("close", () => {
      this.clients.delete(ws);
    });
    ws.send(JSON.stringify({
      jsonrpc: "2.0",
      method: "$/oxplow/status",
      params: { message: null },
    }));
  }

  get languageId(): string {
    return this.registration.languageId;
  }

  /** Ensure the given document is open in the server, syncing text if the
   *  tracked copy is stale. Safe to call repeatedly for the same URI. */
  syncDocument(uri: string, text: string): void {
    const existing = this.openDocuments.get(uri);
    if (!existing) {
      this.openDocuments.set(uri, { version: 1, text });
      this.notifyServer("textDocument/didOpen", {
        textDocument: {
          uri,
          languageId: this.registration.languageId,
          version: 1,
          text,
        },
      });
      return;
    }
    if (existing.text === text) return;
    const nextVersion = existing.version + 1;
    this.openDocuments.set(uri, { version: nextVersion, text });
    this.notifyServer("textDocument/didChange", {
      textDocument: { uri, version: nextVersion },
      contentChanges: [{ text }],
    });
  }

  closeDocument(uri: string): void {
    if (!this.openDocuments.has(uri)) return;
    this.openDocuments.delete(uri);
    this.diagnosticsByUri.delete(uri);
    this.notifyServer("textDocument/didClose", { textDocument: { uri } });
  }

  request<T = unknown>(method: string, params?: unknown): Promise<T> {
    return this.requestToServer(method, params) as Promise<T>;
  }

  getDiagnostics(uri: string): LspDiagnostic[] | undefined {
    return this.diagnosticsByUri.get(uri);
  }

  /** Wait until the server has published diagnostics for `uri`, or until
   *  `timeoutMs` elapses. Returns whatever is cached at that point (possibly
   *  an empty array). */
  async waitForDiagnostics(uri: string, timeoutMs: number): Promise<LspDiagnostic[]> {
    const cached = this.diagnosticsByUri.get(uri);
    if (cached) return cached;
    return new Promise<LspDiagnostic[]>((resolve) => {
      const waiters = this.diagnosticsWaiters.get(uri) ?? [];
      const timer = setTimeout(() => {
        const index = waiters.indexOf(waiter);
        if (index >= 0) waiters.splice(index, 1);
        resolve(this.diagnosticsByUri.get(uri) ?? []);
      }, timeoutMs);
      const waiter = (diagnostics: LspDiagnostic[]) => {
        clearTimeout(timer);
        resolve(diagnostics);
      };
      waiters.push(waiter);
      this.diagnosticsWaiters.set(uri, waiters);
    });
  }

  async dispose(): Promise<void> {
    const proc = this.proc;
    this.proc = null;
    this.initialized = false;
    if (!proc) return;
    try {
      this.notifyServer("exit");
    } catch {}
    proc.kill();
    await new Promise<void>((resolve) => {
      proc.once("exit", () => resolve());
      setTimeout(resolve, 500);
    });
  }

  private forwardRequest(ws: BridgeSocket, message: JsonRpcRequest): void {
    const id = this.nextServerId++;
    this.pending.set(id, { ws, clientId: message.id! });
    this.sendToServer({
      jsonrpc: "2.0",
      id,
      method: message.method,
      params: message.params,
    });
  }

  private notifyServer(method: string, params?: unknown): void {
    this.sendToServer({
      jsonrpc: "2.0",
      method,
      params,
    });
  }

  private async requestToServer(method: string, params?: unknown): Promise<unknown> {
    const id = this.nextServerId++;
    const response = await new Promise<JsonRpcResponse>((resolve, reject) => {
      const fakeWs = {
        send: (body: string) => {
          try {
            resolve(JSON.parse(body) as JsonRpcResponse);
          } catch (error) {
            reject(error);
          }
        },
      } as unknown as BridgeSocket;
      this.pending.set(id, { ws: fakeWs, clientId: id });
      this.sendToServer({
        jsonrpc: "2.0",
        id,
        method,
        params,
      });
    });
    if (response.error) throw new Error(response.error.message);
    return response.result;
  }

  private sendToServer(message: JsonRpcMessage): void {
    if (!this.proc?.stdin.writable) {
      throw new Error(`language server not running for ${this.registration.languageId}`);
    }
    const body = JSON.stringify(message);
    this.proc.stdin.write(`Content-Length: ${Buffer.byteLength(body, "utf8")}\r\n\r\n${body}`);
  }

  private readChunk(chunk: string): void {
    this.buffer += chunk;
    while (true) {
      if (this.contentLength == null) {
        const headerEnd = this.buffer.indexOf("\r\n\r\n");
        if (headerEnd < 0) return;
        const header = this.buffer.slice(0, headerEnd);
        this.buffer = this.buffer.slice(headerEnd + 4);
        const match = header.match(/Content-Length:\s*(\d+)/i);
        if (!match) continue;
        this.contentLength = Number(match[1]);
      }
      if (this.buffer.length < this.contentLength) return;
      const payload = this.buffer.slice(0, this.contentLength);
      this.buffer = this.buffer.slice(this.contentLength);
      this.contentLength = null;
      const parsed = parseJsonRpc(payload);
      if (parsed) this.handleServerMessage(parsed);
    }
  }

  private handleServerMessage(message: JsonRpcMessage): void {
    if ("method" in message) {
      if (message.id !== undefined) {
        this.handleServerRequest(message);
      } else {
        if (message.method === "textDocument/publishDiagnostics") {
          this.recordPublishedDiagnostics(message.params);
        }
        this.broadcast(message);
      }
      return;
    }
    const pending = this.pending.get(message.id);
    if (!pending) return;
    this.pending.delete(message.id);
    pending.ws.send(JSON.stringify({
      ...message,
      id: pending.clientId,
    }));
  }

  private handleServerRequest(message: JsonRpcRequest): void {
    switch (message.method) {
      case "workspace/configuration":
        this.respondToServer(message.id!, []);
        return;
      case "workspace/workspaceFolders":
        this.respondToServer(message.id!, [
          {
            uri: fileUri(this.stream.worktree_path),
            name: this.stream.title,
          },
        ]);
        return;
      case "client/registerCapability":
      case "client/unregisterCapability":
      case "window/workDoneProgress/create":
        this.respondToServer(message.id!, null);
        return;
      default:
        this.respondToServer(message.id!, null);
    }
  }

  private recordPublishedDiagnostics(params: unknown): void {
    if (!params || typeof params !== "object") return;
    const payload = params as { uri?: unknown; diagnostics?: unknown };
    if (typeof payload.uri !== "string" || !Array.isArray(payload.diagnostics)) return;
    const diagnostics = payload.diagnostics as LspDiagnostic[];
    this.diagnosticsByUri.set(payload.uri, diagnostics);
    const waiters = this.diagnosticsWaiters.get(payload.uri);
    if (waiters && waiters.length > 0) {
      this.diagnosticsWaiters.delete(payload.uri);
      for (const waiter of waiters) waiter(diagnostics);
    }
  }

  private respondToServer(id: number | string, result: unknown): void {
    this.sendToServer({
      jsonrpc: "2.0",
      id,
      result,
    });
  }

  private broadcast(message: JsonRpcRequest): void {
    const body = JSON.stringify(message);
    for (const client of this.clients) {
      client.send(body);
    }
  }
}

function registrationForLanguage(languageId: string): LanguageServerRegistration | undefined {
  return REGISTRY.find((candidate) => candidate.languageId === languageId);
}

function parseJsonRpc(raw: string): JsonRpcMessage | null {
  try {
    return JSON.parse(raw) as JsonRpcMessage;
  } catch {
    return null;
  }
}

export function fileUri(path: string): string {
  return `file://${encodeURI(path.replace(/\\/g, "/"))}`;
}
