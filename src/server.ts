import { createServer, IncomingMessage, ServerResponse } from "node:http";
import { readFileSync, existsSync, statSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { WebSocketServer } from "ws";
import { attachPane } from "./pty-bridge.js";
import { ensureStreamPane } from "./fleet.js";
import { ensureWorktree, listBranches } from "./git.js";
import { HookEventStore, ingestHookPayload } from "./hook-ingest.js";
import type { PaneKind, Stream, StreamStore } from "./stream-store.js";

interface Deps {
  store: StreamStore;
  publicDir: string;
  projectDir: string;
  projectBase: string;
  port: number;
  hookEvents: HookEventStore;
  getClaudeCommand(stream: Stream, pane: PaneKind): string;
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
    const resolvedPane = pane ? deps.store.findByPane(pane) : undefined;
    if (!pane || !resolvedPane || cols < 2 || rows < 2) {
      socket.destroy();
      return;
    }
    try {
      ensureStreamPane(
        resolvedPane.stream,
        resolvedPane.pane,
        cols,
        rows,
        deps.getClaudeCommand(resolvedPane.stream, resolvedPane.pane),
      );
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

function handleHttp(req: IncomingMessage, res: ServerResponse, deps: Deps) {
  const url = new URL(req.url ?? "/", "http://localhost");
  const path = url.pathname;

  if (path === "/api/streams" && req.method === "GET") {
    return json(res, 200, deps.store.list());
  }
  if (path === "/api/streams/current" && req.method === "GET") {
    const s = deps.store.getCurrent();
    if (!s) return json(res, 404, { error: "no current stream" });
    return json(res, 200, s);
  }
  if (path === "/api/streams/current" && req.method === "POST") {
    return readBody(req, (body) => {
      const parsed = parseJsonBody(body);
      if (!parsed || typeof parsed.id !== "string") {
        return json(res, 400, { error: "expected { id }" });
      }
      try {
        deps.store.setCurrentStreamId(parsed.id);
      } catch (e) {
        return json(res, 404, { error: (e as Error).message });
      }
      return json(res, 200, deps.store.get(parsed.id));
    });
  }
  if (path === "/api/streams/current" && req.method === "PUT") {
    return readBody(req, (body) => {
      const parsed = parseJsonBody(body);
      if (!parsed || typeof parsed.title !== "string" || !parsed.title.trim()) {
        return json(res, 400, { error: "title is required" });
      }
      const current = deps.store.getCurrent();
      if (!current) return json(res, 404, { error: "no current stream" });
      const updated = deps.store.update(current.id, (stream) => ({
        ...stream,
        title: parsed.title.trim(),
      }));
      return json(res, 200, updated);
    });
  }
  if (path === "/api/branches" && req.method === "GET") {
    return json(res, 200, listBranches(deps.projectDir));
  }
  if (path === "/api/streams" && req.method === "POST") {
    return readBody(req, (body) => handleCreateStream(req, res, deps, parseJsonBody(body)));
  }
  if (path.startsWith("/api/hook/") && req.method === "POST") {
    const event = path.slice("/api/hook/".length);
    const streamId = url.searchParams.get("stream") ?? deps.store.getCurrentStreamId() ?? "default";
    const pane = parsePane(url.searchParams.get("pane"));
    return readBody(req, (body) => {
      let payload: any = null;
      try { payload = body ? JSON.parse(body) : {}; } catch { payload = { raw: body }; }
      const stored = ingestHookPayload(deps.hookEvents, event, payload, { streamId, pane });
      if (pane && stored.normalized.sessionId && deps.store.get(streamId)) {
        deps.store.update(streamId, (stream) => ({
          ...stream,
          resume: pane === "working"
            ? { ...stream.resume, working_session_id: stored.normalized.sessionId ?? "" }
            : { ...stream.resume, talking_session_id: stored.normalized.sessionId ?? "" },
        }));
      }
      return json(res, 200, { ok: true });
    });
  }
  if (path === "/api/hooks" && req.method === "GET") {
    const streamId = resolveStreamSelector(url.searchParams.get("stream"), deps.store.getCurrentStreamId() ?? undefined);
    return json(res, 200, deps.hookEvents.list(streamId));
  }
  if (path === "/api/hooks/stream" && req.method === "GET") {
    const streamId = resolveStreamSelector(url.searchParams.get("stream"), deps.store.getCurrentStreamId() ?? undefined);
    return handleHookStream(res, deps, streamId);
  }

  return serveStatic(path, res, deps.publicDir);
}

function handleCreateStream(_req: IncomingMessage, res: ServerResponse, deps: Deps, body: any) {
  if (!body || typeof body.title !== "string" || !body.title.trim()) {
    return json(res, 400, { error: "title is required" });
  }
  const title = body.title.trim();
  const summary = typeof body.summary === "string" ? body.summary.trim() : "";
  const source = body.source;
  try {
    let stream: Stream;
    if (source === "existing") {
      if (typeof body.ref !== "string") return json(res, 400, { error: "ref is required" });
      const branch = listBranches(deps.projectDir).find((candidate) => candidate.ref === body.ref);
      if (!branch) return json(res, 404, { error: `unknown branch ref: ${body.ref}` });
      const localBranch = branch.kind === "local" ? branch.name : localBranchName(branch.name);
      const existing = deps.store.findByBranch(localBranch);
      if (existing) {
        deps.store.setCurrentStreamId(existing.id);
        return json(res, 200, existing);
      }
      const worktreePath = streamWorktreePath(deps.projectDir, localBranch);
      ensureWorktree(
        deps.projectDir,
        worktreePath,
        branch.kind === "local"
          ? { kind: "existing-local", branch: localBranch }
          : { kind: "existing-remote", branch: localBranch, remoteRef: branch.name },
      );
      stream = deps.store.create({
        title,
        summary,
        branch: localBranch,
        branchRef: branch.ref,
        branchSource: branch.kind,
        worktreePath,
        projectBase: deps.projectBase,
      });
    } else if (source === "new") {
      if (typeof body.branch !== "string" || !body.branch.trim()) {
        return json(res, 400, { error: "branch is required" });
      }
      if (typeof body.startPointRef !== "string" || !body.startPointRef.trim()) {
        return json(res, 400, { error: "startPointRef is required" });
      }
      const branchName = body.branch.trim();
      const existing = deps.store.findByBranch(branchName);
      if (existing) {
        deps.store.setCurrentStreamId(existing.id);
        return json(res, 200, existing);
      }
      const worktreePath = streamWorktreePath(deps.projectDir, branchName);
      ensureWorktree(deps.projectDir, worktreePath, {
        kind: "new",
        branch: branchName,
        startPoint: body.startPointRef,
      });
      stream = deps.store.create({
        title,
        summary,
        branch: branchName,
        branchRef: body.startPointRef,
        branchSource: "new",
        worktreePath,
        projectBase: deps.projectBase,
      });
    } else {
      return json(res, 400, { error: "source must be 'existing' or 'new'" });
    }
    deps.store.setCurrentStreamId(stream.id);
    return json(res, 201, stream);
  } catch (e) {
    return json(res, 400, { error: (e as Error).message });
  }
}

function handleHookStream(res: ServerResponse, deps: Deps, streamId?: string) {
  res.writeHead(200, {
    "content-type": "text/event-stream",
    "cache-control": "no-cache, no-transform",
    connection: "keep-alive",
    "x-accel-buffering": "no",
  });
  // Replay existing buffer so a fresh client gets recent history.
  for (const evt of deps.hookEvents.list(streamId)) {
    res.write(`data: ${JSON.stringify(evt)}\n\n`);
  }
  const unsub = deps.hookEvents.subscribe((evt) => {
    res.write(`data: ${JSON.stringify(evt)}\n\n`);
  }, streamId);
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

function parseJsonBody(body: string): any {
  try {
    return body ? JSON.parse(body) : {};
  } catch {
    return null;
  }
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

function streamWorktreePath(projectDir: string, branch: string): string {
  return join(projectDir, ".newde", "worktrees", sanitizeBranch(branch));
}

function sanitizeBranch(branch: string): string {
  return branch.replace(/[^a-zA-Z0-9._-]+/g, "-");
}

function localBranchName(remoteName: string): string {
  const slash = remoteName.indexOf("/");
  return slash >= 0 ? remoteName.slice(slash + 1) : remoteName;
}

function parsePane(value: string | null): PaneKind | undefined {
  return value === "working" || value === "talking" ? value : undefined;
}

function resolveStreamSelector(value: string | null, fallback?: string): string | undefined {
  if (value === "all") return undefined;
  return value ?? fallback;
}
