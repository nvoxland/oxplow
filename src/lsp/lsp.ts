import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { existsSync } from "node:fs";
import { resolve } from "node:path";
import type WebSocket from "ws";
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

interface LanguageServerRegistration {
  languageId: string;
  extensions: string[];
  command: string;
  args: string[];
}

interface PendingRequest {
  ws: WebSocket;
  clientId: number | string;
}

const TYPESCRIPT_LANGUAGE_SERVER = resolve(new URL("..", import.meta.url).pathname, "node_modules", ".bin", "typescript-language-server");

const REGISTRY: LanguageServerRegistration[] = [
  {
    languageId: "typescript",
    extensions: [".ts", ".tsx"],
    command: TYPESCRIPT_LANGUAGE_SERVER,
    args: ["--stdio"],
  },
  {
    languageId: "javascript",
    extensions: [".js", ".jsx", ".mjs", ".cjs"],
    command: TYPESCRIPT_LANGUAGE_SERVER,
    args: ["--stdio"],
  },
];

export class LspSessionManager {
  private sessions = new Map<string, LspSession>();

  constructor(private readonly logger: Logger) {}

  async attachClient(ws: WebSocket, stream: Stream, languageId: string): Promise<void> {
    const session = await this.ensureSession(stream, languageId);
    session.attachClient(ws);
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

class LspSession {
  private proc: ChildProcessWithoutNullStreams | null = null;
  private buffer = "";
  private contentLength: number | null = null;
  private nextServerId = 1;
  private initialized = false;
  private clients = new Set<WebSocket>();
  private pending = new Map<number | string, PendingRequest>();

  constructor(
    private readonly stream: Stream,
    private readonly registration: LanguageServerRegistration,
    private readonly logger: Logger,
  ) {}

  async initialize(): Promise<void> {
    if (this.initialized) return;
    if (!existsSync(this.registration.command)) {
      throw new Error(`missing language server executable: ${this.registration.command}`);
    }
    const proc = spawn(this.registration.command, this.registration.args, {
      cwd: this.stream.worktree_path,
      stdio: "pipe",
      env: process.env,
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
        method: "$/newde/status",
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

  attachClient(ws: WebSocket): void {
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
      method: "$/newde/status",
      params: { message: null },
    }));
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

  private forwardRequest(ws: WebSocket, message: JsonRpcRequest): void {
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
      } as unknown as WebSocket;
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

function fileUri(path: string): string {
  return `file://${encodeURI(path.replace(/\\/g, "/"))}`;
}
