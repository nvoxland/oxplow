import { createServer, IncomingMessage, ServerResponse } from "node:http";
import { randomBytes, timingSafeEqual } from "node:crypto";
import { writeFileSync, unlinkSync, readdirSync, readFileSync, existsSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { WebSocketServer, WebSocket } from "ws";
import type { Logger } from "../core/logger.js";

const PROTOCOL_VERSION = "2024-11-05";
const SERVER_NAME = "newde";
const SERVER_VERSION = "0.0.1";
const HTTP_AUTH_HEADER = "authorization";
const MAX_HTTP_BODY_BYTES = 1_000_000;

export interface HookEnvelope {
  event: string;
  streamId?: string;
  batchId?: string;
  pane?: string;
  payload: unknown;
}

export interface HookHandlerResponse {
  /** Optional HTTP status override. Defaults to 200 when a body is returned, 202 otherwise. */
  status?: number;
  /** JSON body to send back to Claude Code (e.g. hookSpecificOutput.additionalContext). */
  body?: unknown;
}

interface StartOptions {
  workspaceFolders: string[];
  /** Directory where the `<port>.lock` IDE discovery file is written.
   *  Defaults to `~/.claude/ide` so Claude Code's normal auto-discovery
   *  finds us. Overridable for tests. */
  ideDir?: string;
  logger?: Logger;
  extraTools?: ToolDef[];
  /** Optional hook-forwarder handler. Called for each POST to
   *  `/hook/:event` with a parsed envelope; the agent shim is wired
   *  to this endpoint so hook events reach the runtime without a
   *  filesystem inbox. */
  onHook?: (envelope: HookEnvelope) => void | HookHandlerResponse | Promise<void | HookHandlerResponse>;
}

export interface McpServerHandle {
  port: number;
  authToken: string;
  httpUrl: string;
  /** Base URL hook forwarders POST to; event name goes on the path. */
  hookUrl: string;
  lockfilePath: string;
  stop(): Promise<void>;
}

interface RpcRequest {
  jsonrpc: "2.0";
  id?: number | string;
  method: string;
  params?: any;
}

export interface ToolDef {
  name: string;
  description: string;
  inputSchema: { type: "object"; properties: Record<string, any>; required?: string[] };
  handler: (args: any) => any;
}

export async function startMcpServer(opts: StartOptions): Promise<McpServerHandle> {
  const logger = opts.logger;
  const ideDir = opts.ideDir ?? join(homedir(), ".claude", "ide");
  mkdirSync(ideDir, { recursive: true });
  sweepStaleLockfiles(ideDir);

  const authToken = randomBytes(32).toString("base64url");
  const http = createServer((req, res) => {
    void handleHttpRequest(req, res);
  });
  const wss = new WebSocketServer({ server: http });
  wss.on("connection", (ws, req) => {
    if (!isAuthorizedHttpRequest(req, authToken)) {
      // debug, not warn — unauthorized probes from other IDEs scanning
      // `~/.claude/ide/*.lock` fire this on every newde launch. The
      // rejection itself is correct defense; surfacing it at WARN just
      // pollutes startup logs with expected behavior.
      logger?.debug("rejected unauthorized mcp websocket");
      ws.close(1008, "unauthorized");
      return;
    }
    handleConnection(ws);
  });

  const tools: ToolDef[] = [];
  let serverPort = 0;

  tools.push({
    name: "newde__ping",
    description: "Proof-of-life tool. Returns ok + daemonPort + timestamp.",
    inputSchema: { type: "object", properties: {} },
    handler: () => ({ ok: true, daemonPort: serverPort, timestamp: Date.now() }),
  });
  if (opts.extraTools?.length) {
    tools.push(...opts.extraTools);
  }

  async function handleHttpRequest(req: IncomingMessage, res: ServerResponse) {
    const url = new URL(req.url ?? "/", "http://127.0.0.1");
    if (url.pathname.startsWith("/hook/")) {
      return handleHookRequest(req, res, url);
    }
    if (url.pathname !== "/mcp") {
      res.statusCode = 404;
      res.end("not found");
      return;
    }
    if (!isAuthorizedHttpRequest(req, authToken)) {
      logger?.warn("rejected unauthorized mcp http request");
      res.statusCode = 401;
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify({ error: "unauthorized" }));
      return;
    }
    if (req.method === "OPTIONS") {
      res.statusCode = 204;
      res.setHeader("access-control-allow-origin", "*");
      res.setHeader("access-control-allow-headers", "content-type, authorization");
      res.setHeader("access-control-allow-methods", "POST, OPTIONS");
      res.end();
      return;
    }
    if (req.method !== "POST") {
      res.statusCode = 405;
      res.setHeader("allow", "POST, OPTIONS");
      res.end("method not allowed");
      return;
    }

    let body: string;
    try {
      body = await readRequestBody(req, MAX_HTTP_BODY_BYTES);
    } catch (error) {
      if (error instanceof BodyTooLargeError) {
        res.statusCode = 413;
        res.setHeader("content-type", "application/json");
        res.end(JSON.stringify({ error: "payload too large" }));
        return;
      }
      throw error;
    }

    let payload: unknown;
    try {
      payload = JSON.parse(body);
    } catch {
      res.statusCode = 400;
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify({ error: "invalid json" }));
      return;
    }

    const rawBatch = Array.isArray(payload) ? payload : [payload];
    const candidates = rawBatch
      .map((entry) => entry as RpcRequest)
      .filter((entry): entry is RpcRequest => !!entry && typeof entry === "object");
    const responses: Array<Record<string, unknown>> = [];
    for (const candidate of candidates) {
      const result = dispatch(candidate);
      const isNotification = candidate.method === "notifications/initialized" || candidate.id === undefined;
      if (isNotification) continue;
      responses.push({ jsonrpc: "2.0", id: candidate.id, ...result });
    }

    if (responses.length === 0) {
      res.statusCode = 202;
      res.end();
      return;
    }

    res.statusCode = 200;
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify(Array.isArray(payload) ? responses : responses[0]));
  }

  async function handleHookRequest(req: IncomingMessage, res: ServerResponse, url: URL) {
    if (!isAuthorizedHttpRequest(req, authToken)) {
      logger?.warn("rejected unauthorized hook http request");
      res.statusCode = 401;
      res.end();
      return;
    }
    if (req.method !== "POST") {
      res.statusCode = 405;
      res.setHeader("allow", "POST");
      res.end();
      return;
    }
    const event = url.pathname.slice("/hook/".length);
    if (!event) {
      res.statusCode = 400;
      res.end("missing event name");
      return;
    }

    let body: string;
    try {
      body = await readRequestBody(req, MAX_HTTP_BODY_BYTES);
    } catch (error) {
      if (error instanceof BodyTooLargeError) {
        res.statusCode = 413;
        res.end();
        return;
      }
      throw error;
    }

    let payload: unknown = null;
    if (body) {
      try { payload = JSON.parse(body); } catch { payload = { raw: body }; }
    }

    // Identity rides X-Newde-* request headers (Claude's http hooks support
    // env-var interpolation in header values).
    const envelope: HookEnvelope = {
      event,
      streamId: readHeader(req, "x-newde-stream"),
      batchId: readHeader(req, "x-newde-batch"),
      pane: readHeader(req, "x-newde-pane"),
      payload,
    };

    let response: HookHandlerResponse | void = undefined;
    try {
      response = await opts.onHook?.(envelope);
    } catch (error) {
      logger?.warn("hook handler threw", {
        event,
        error: error instanceof Error ? error.message : String(error),
      });
    }

    if (response && response.body !== undefined) {
      res.statusCode = response.status ?? 200;
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify(response.body));
      return;
    }
    res.statusCode = response?.status ?? 202;
    res.end();
  }

  function handleConnection(ws: WebSocket) {
    ws.on("message", (raw) => {
      let req: RpcRequest;
      try {
        req = JSON.parse(raw.toString());
      } catch {
        logger?.warn("failed to parse mcp rpc request");
        return;
      }
      if (req.method === "notifications/initialized") return; // no response
      const result = dispatch(req);
      if (req.id === undefined) return;
      ws.send(JSON.stringify({ jsonrpc: "2.0", id: req.id, ...result }));
    });
  }

  function dispatch(req: RpcRequest): { result?: any; error?: { code: number; message: string } } {
    try {
      switch (req.method) {
        case "initialize":
          return {
            result: {
              protocolVersion: PROTOCOL_VERSION,
              capabilities: { tools: {} },
              serverInfo: { name: SERVER_NAME, version: SERVER_VERSION },
            },
          };
        case "tools/list":
          return {
            result: {
              tools: tools.map((t) => ({
                name: t.name,
                description: t.description,
                inputSchema: t.inputSchema,
              })),
            },
          };
        case "tools/call": {
          const name = req.params?.name;
          const args = req.params?.arguments ?? {};
          const tool = tools.find((t) => t.name === name);
          if (!tool) {
            return { error: { code: -32601, message: `unknown tool: ${name}` } };
          }
          const validation = validateToolArgs(tool, args);
          if (validation) {
            return { error: { code: -32602, message: `invalid params for ${name}: ${validation}` } };
          }
          const out = tool.handler(args);
          return {
            result: {
              content: [{ type: "text", text: JSON.stringify(out) }],
              isError: false,
            },
          };
        }
        default:
          return { error: { code: -32601, message: `method not found: ${req.method}` } };
      }
    } catch (e) {
      return { error: { code: -32603, message: (e as Error).message } };
    }
  }

  await new Promise<void>((resolve, reject) => {
    http.once("error", reject);
    http.listen(0, "127.0.0.1", () => {
      http.off("error", reject);
      resolve();
    });
  });

  const addr = http.address();
  if (!addr || typeof addr === "string") {
    throw new Error("mcp server: unexpected listen address");
  }
  serverPort = addr.port;
  const httpUrl = `http://127.0.0.1:${serverPort}/mcp`;
  const hookUrl = `http://127.0.0.1:${serverPort}/hook`;

  const lockfilePath = join(ideDir, `${serverPort}.lock`);
  const lockBody = {
    pid: process.pid,
    port: serverPort,
    workspaceFolders: opts.workspaceFolders,
    ideName: SERVER_NAME,
    transport: "ws" as const,
    runningInWindows: process.platform === "win32",
    authToken,
  };
  writeFileSync(lockfilePath, JSON.stringify(lockBody), "utf8");
  logger?.info("mcp server listening", { port: serverPort, httpUrl, lockfilePath, workspaceFolders: opts.workspaceFolders });

  async function stop() {
    try {
      if (existsSync(lockfilePath)) unlinkSync(lockfilePath);
    } catch {}
    // Evict Claude's hook connections: wss.close / http.close would otherwise
    // wait for idle keep-alive sockets (Node's default is 5 s) before firing
    // their callbacks, stalling app shutdown.
    for (const client of wss.clients) {
      try { client.terminate(); } catch {}
    }
    await new Promise<void>((resolve) => {
      wss.close(() => resolve());
    });
    http.closeAllConnections?.();
    await new Promise<void>((resolve) => {
      http.close(() => resolve());
    });
    logger?.info("mcp server stopped", { port: serverPort });
  }

  return { port: serverPort, authToken, httpUrl, hookUrl, lockfilePath, stop };
}

function readHeader(req: IncomingMessage, name: string): string | undefined {
  const value = req.headers[name];
  if (typeof value === "string" && value.length > 0) return value;
  if (Array.isArray(value) && value.length > 0 && value[0]) return value[0];
  return undefined;
}

function isAuthorizedHttpRequest(req: IncomingMessage, authToken: string): boolean {
  const authorization = req.headers[HTTP_AUTH_HEADER];
  if (typeof authorization !== "string") return false;
  return constantTimeStringEqual(authorization, `Bearer ${authToken}`);
}

function constantTimeStringEqual(a: string, b: string): boolean {
  // Length-first short-circuit is fine — leaks length but not content. The
  // token itself is fixed-length so the prefix and suffix lengths don't
  // vary across legitimate calls.
  if (a.length !== b.length) return false;
  const aBuf = Buffer.from(a, "utf8");
  const bBuf = Buffer.from(b, "utf8");
  return timingSafeEqual(aBuf, bBuf);
}

class BodyTooLargeError extends Error {
  constructor(limit: number) {
    super(`request body exceeds ${limit} bytes`);
    this.name = "BodyTooLargeError";
  }
}

function readRequestBody(req: IncomingMessage, maxBytes: number): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let total = 0;
    req.on("data", (chunk) => {
      const buf = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
      total += buf.length;
      if (total > maxBytes) {
        req.destroy();
        reject(new BodyTooLargeError(maxBytes));
        return;
      }
      chunks.push(buf);
    });
    req.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    req.on("error", reject);
  });
}

function sweepStaleLockfiles(ideDir: string): void {
  if (!existsSync(ideDir)) return;
  for (const name of readdirSync(ideDir)) {
    if (!name.endsWith(".lock")) continue;
    const path = join(ideDir, name);
    try {
      const body = JSON.parse(readFileSync(path, "utf8"));
      if (typeof body.pid !== "number" || !isPidAlive(body.pid)) {
        unlinkSync(path);
      }
    } catch {
      try {
        unlinkSync(path);
      } catch {}
    }
  }
}

function isPidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e: any) {
    return e.code === "EPERM";
  }
}

/**
 * Validate `args` against the tool's `inputSchema` (a small subset of JSON
 * Schema: required fields + per-property types). Returns null on success,
 * a human-readable error message on failure. Deliberately tiny — the tool
 * schemas in this project use only `string` / `number` / `boolean` /
 * `object` / `array` and `required` lists; if a future tool needs richer
 * validation, swap in Ajv.
 */
export function validateToolArgs(tool: ToolDef, args: unknown): string | null {
  if (typeof args !== "object" || args === null || Array.isArray(args)) {
    return "arguments must be an object";
  }
  const a = args as Record<string, unknown>;
  for (const requiredKey of tool.inputSchema.required ?? []) {
    if (!(requiredKey in a)) return `missing required field: ${requiredKey}`;
  }
  for (const [key, spec] of Object.entries(tool.inputSchema.properties)) {
    if (!(key in a)) continue;
    const expected = (spec as { type?: string }).type;
    if (!expected) continue;
    const actual = jsonTypeOf(a[key]);
    if (actual !== expected) {
      return `field ${key} should be ${expected} but got ${actual}`;
    }
  }
  return null;
}

function jsonTypeOf(value: unknown): string {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  return typeof value;
}
