import { createServer, IncomingMessage } from "node:http";
import { randomBytes } from "node:crypto";
import { writeFileSync, unlinkSync, readdirSync, readFileSync, existsSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { WebSocketServer, WebSocket } from "ws";
import type { Logger } from "./logger.js";

const PROTOCOL_VERSION = "2024-11-05";
const SERVER_NAME = "newde";
const SERVER_VERSION = "0.0.1";
const AUTH_HEADER = "x-claude-code-ide-authorization";

interface StartOptions {
  workspaceFolders: string[];
  /** Directory where the `<port>.lock` IDE discovery file is written.
   *  Defaults to `~/.claude/ide` so Claude Code's normal auto-discovery
   *  finds us. Overridable for tests. */
  ideDir?: string;
  logger?: Logger;
}

export interface McpServerHandle {
  port: number;
  authToken: string;
  lockfilePath: string;
  stop(): Promise<void>;
}

interface RpcRequest {
  jsonrpc: "2.0";
  id?: number | string;
  method: string;
  params?: any;
}

interface ToolDef {
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
  const http = createServer();
  const wss = new WebSocketServer({ server: http });
  wss.on("connection", (ws, req) => {
    const presented = req.headers[AUTH_HEADER];
    if (presented !== authToken) {
      logger?.warn("rejected unauthorized mcp websocket");
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
  logger?.info("mcp server listening", { port: serverPort, lockfilePath, workspaceFolders: opts.workspaceFolders });

  async function stop() {
    try {
      if (existsSync(lockfilePath)) unlinkSync(lockfilePath);
    } catch {}
    await new Promise<void>((resolve) => {
      wss.close(() => resolve());
    });
    await new Promise<void>((resolve) => {
      http.close(() => resolve());
    });
    logger?.info("mcp server stopped", { port: serverPort });
  }

  return { port: serverPort, authToken, lockfilePath, stop };
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
