import { createServer, IncomingMessage, ServerResponse } from "node:http";
import { readFileSync, existsSync, statSync } from "node:fs";
import { extname, resolve } from "node:path";
import { WebSocketServer } from "ws";
import { attachPane } from "./pty-bridge.js";
import { ensureStreamPane } from "./fleet.js";
import type { StreamStore } from "./stream-store.js";
import { HookEventStore, ingestHookPayload } from "./hook-ingest.js";

interface Deps {
  store: StreamStore;
  currentStreamId: string;
  publicDir: string;
  projectDir: string;
  port: number;
  claudeCommand: string;
  hookEvents: HookEventStore;
}

const MIME: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".js": "application/javascript; charset=utf-8",
  ".mjs": "application/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
};

export function startServer(deps: Deps) {
  const http = createServer((req, res) => handleHttp(req, res, deps));
  const wss = new WebSocketServer({ noServer: true });

  http.on("upgrade", (req, socket, head) => {
    const url = new URL(req.url ?? "", "http://localhost");
    if (url.pathname !== "/ws") {
      socket.destroy();
      return;
    }
    const pane = url.searchParams.get("pane");
    const cols = Number(url.searchParams.get("cols") ?? "0");
    const rows = Number(url.searchParams.get("rows") ?? "0");
    if (!pane || !validatePane(pane, deps.store) || cols < 2 || rows < 2) {
      socket.destroy();
      return;
    }
    try {
      ensureStreamPane(pane, deps.projectDir, cols, rows, deps.claudeCommand);
    } catch (e) {
      console.warn(`[newde] ensureStreamPane failed:`, e);
      socket.destroy();
      return;
    }
    wss.handleUpgrade(req, socket, head, (ws) => {
      attachPane(ws, pane, cols, rows);
    });
  });

  http.listen(deps.port, "127.0.0.1", () => {
    console.log(`[newde] http://127.0.0.1:${deps.port}`);
  });
}

function validatePane(target: string, store: StreamStore): boolean {
  for (const s of store.list()) {
    if (s.panes.working === target || s.panes.talking === target) return true;
  }
  return false;
}

function handleHttp(req: IncomingMessage, res: ServerResponse, deps: Deps) {
  const url = new URL(req.url ?? "/", "http://localhost");
  const path = url.pathname;

  if (path === "/api/streams" && req.method === "GET") {
    return json(res, 200, deps.store.list());
  }
  if (path === "/api/streams/current" && req.method === "GET") {
    const s = deps.store.get(deps.currentStreamId);
    if (!s) return json(res, 404, { error: "no current stream" });
    return json(res, 200, s);
  }
  if (path === "/api/streams" && req.method === "POST") {
    return json(res, 501, { error: "not implemented" });
  }
  if (path.startsWith("/api/hook/") && req.method === "POST") {
    const event = path.slice("/api/hook/".length);
    return readBody(req, (body) => {
      let payload: any = null;
      try { payload = body ? JSON.parse(body) : {}; } catch { payload = { raw: body }; }
      ingestHookPayload(deps.hookEvents, event, payload);
      return json(res, 200, { ok: true });
    });
  }
  if (path === "/api/hooks" && req.method === "GET") {
    return json(res, 200, deps.hookEvents.list());
  }
  if (path === "/api/hooks/stream" && req.method === "GET") {
    return handleHookStream(res, deps);
  }

  return serveStatic(path, res, deps.publicDir);
}

function handleHookStream(res: ServerResponse, deps: Deps) {
  res.writeHead(200, {
    "content-type": "text/event-stream",
    "cache-control": "no-cache, no-transform",
    connection: "keep-alive",
    "x-accel-buffering": "no",
  });
  // Replay existing buffer so a fresh client gets recent history.
  for (const evt of deps.hookEvents.list()) {
    res.write(`data: ${JSON.stringify(evt)}\n\n`);
  }
  const unsub = deps.hookEvents.subscribe((evt) => {
    res.write(`data: ${JSON.stringify(evt)}\n\n`);
  });
  const keepalive = setInterval(() => res.write(": ping\n\n"), 15000);
  res.on("close", () => {
    clearInterval(keepalive);
    unsub();
  });
}

function readBody(req: IncomingMessage, cb: (body: string) => void) {
  const chunks: Buffer[] = [];
  req.on("data", (c) => chunks.push(c));
  req.on("end", () => cb(Buffer.concat(chunks).toString("utf8")));
}

function serveStatic(path: string, res: ServerResponse, publicDir: string) {
  const rel = path === "/" ? "/index.html" : path;
  const abs = resolve(publicDir, "." + rel);
  if (!abs.startsWith(publicDir)) {
    res.writeHead(403);
    res.end("forbidden");
    return;
  }
  if (!existsSync(abs) || !statSync(abs).isFile()) {
    res.writeHead(404);
    res.end("not found");
    return;
  }
  const mime = MIME[extname(abs)] ?? "application/octet-stream";
  res.writeHead(200, { "content-type": mime });
  res.end(readFileSync(abs));
}

function json(res: ServerResponse, status: number, body: unknown) {
  res.writeHead(status, { "content-type": "application/json; charset=utf-8" });
  res.end(JSON.stringify(body));
}
