import { createServer, IncomingMessage, ServerResponse } from "node:http";
import { readFileSync, existsSync, statSync } from "node:fs";
import { extname, join, resolve } from "node:path";
import { WebSocketServer } from "ws";
import { createUiClientLogger, type Logger, type LogLevel } from "./logger.js";
import { attachPane } from "./pty-bridge.js";
import { ResumeTracker } from "./resume-tracker.js";
import { ensureStreamPane } from "./fleet.js";
import { ensureWorktree, isGitRepo, listBranches, listGitStatuses } from "./git.js";
import { HookEventStore, ingestHookPayload } from "./hook-ingest.js";
import { LspSessionManager } from "./lsp.js";
import type { PaneKind, Stream, StreamStore } from "./stream-store.js";
import { listWorkspaceEntries, listWorkspaceFiles, readWorkspaceFile, summarizeGitStatuses, writeWorkspaceFile } from "./workspace-files.js";
import { WorkspaceWatcherRegistry } from "./workspace-watch.js";

interface Deps {
  store: StreamStore;
  publicDir: string;
  projectDir: string;
  projectBase: string;
  port: number;
  hookEvents: HookEventStore;
  workspaceWatchers: WorkspaceWatcherRegistry;
  logger: Logger;
  resumeTracker: ResumeTracker;
  lspManager: LspSessionManager;
  getAgentCommand(stream: Stream, pane: PaneKind): string;
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
    if (url.pathname === "/lsp") {
      const streamId = url.searchParams.get("stream");
      const languageId = url.searchParams.get("language");
      if (!streamId || !languageId) {
        socket.destroy();
        return;
      }
      let stream: Stream;
      try {
        stream = resolveStream(deps, streamId);
      } catch {
        socket.destroy();
        return;
      }
      const lspLogger = deps.logger.child({ subsystem: "lsp", streamId: stream.id, languageId });
      wss.handleUpgrade(req, socket, head, (ws) => {
        void deps.lspManager.attachClient(ws, stream, languageId)
          .then(() => {
            lspLogger.info("accepted lsp websocket");
          })
          .catch((error) => {
            lspLogger.warn("rejected lsp websocket", { error: errorMessage(error) });
            ws.close(1011, errorMessage(error));
          });
      });
      return;
    }
    if (url.pathname !== "/ws") {
      socket.destroy();
      return;
    }
    const pane = url.searchParams.get("pane");
    const cols = Number(url.searchParams.get("cols") ?? "0");
    const rows = Number(url.searchParams.get("rows") ?? "0");
    const resolvedPane = pane ? deps.store.findByPane(pane) : undefined;
    if (!pane || !resolvedPane || cols < 2 || rows < 2) {
      deps.logger.warn("rejected websocket upgrade", {
        paneTarget: pane ?? undefined,
        cols,
        rows,
      });
      socket.destroy();
      return;
    }
    const paneLogger = deps.logger.child({
      streamId: resolvedPane.stream.id,
      pane: resolvedPane.pane,
      paneTarget: pane,
    });
    try {
      const created = ensureStreamPane(
        resolvedPane.stream,
        resolvedPane.pane,
        cols,
        rows,
        deps.getAgentCommand(resolvedPane.stream, resolvedPane.pane),
        paneLogger,
      );
      if (created) {
        const hasResume = resolvedPane.pane === "working"
          ? !!resolvedPane.stream.resume.working_session_id
          : !!resolvedPane.stream.resume.talking_session_id;
        deps.resumeTracker.notePaneLaunch(resolvedPane.stream.id, resolvedPane.pane, hasResume);
      }
    } catch (e) {
      paneLogger.error("failed to ensure stream pane", { error: errorMessage(e) });
      socket.destroy();
      return;
    }
    wss.handleUpgrade(req, socket, head, (ws) => {
      paneLogger.info("accepted pane websocket", { cols, rows });
      attachPane(ws, pane, cols, rows, paneLogger.child({ subsystem: "pty-bridge" }));
    });
  });

  http.listen(deps.port, "127.0.0.1", () => {
    deps.logger.info("http server listening", { port: deps.port });
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
        deps.logger.warn("failed to switch current stream", { streamId: parsed.id, error: errorMessage(e) });
        return json(res, 404, { error: (e as Error).message });
      }
      deps.logger.info("switched current stream", { streamId: parsed.id });
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
      deps.logger.info("renamed current stream", { streamId: updated.id, title: updated.title });
      return json(res, 200, updated);
    });
  }
  if (path === "/api/branches" && req.method === "GET") {
    return json(res, 200, listBranches(deps.projectDir));
  }
  if (path === "/api/workspace/context" && req.method === "GET") {
    return json(res, 200, { gitEnabled: isGitRepo(deps.projectDir) });
  }
  if (path === "/api/workspace/entries" && req.method === "GET") {
    try {
      const stream = resolveStream(deps, url.searchParams.get("stream"));
      const relativePath = url.searchParams.get("path") ?? "";
      const statuses = listGitStatuses(stream.worktree_path);
      deps.logger.debug("listed workspace entries", { streamId: stream.id, path: relativePath });
      return json(res, 200, {
        entries: listWorkspaceEntries(stream.worktree_path, relativePath, statuses),
      });
    } catch (e) {
      deps.logger.warn("failed to list workspace entries", { error: errorMessage(e) });
      return json(res, 400, { error: (e as Error).message });
    }
  }
  if (path === "/api/workspace/files" && req.method === "GET") {
    try {
      const stream = resolveStream(deps, url.searchParams.get("stream"));
      const statuses = listGitStatuses(stream.worktree_path);
      deps.logger.debug("listed workspace files", { streamId: stream.id });
      const files = listWorkspaceFiles(stream.worktree_path, statuses);
      return json(res, 200, {
        files,
        summary: summarizeGitStatuses(statuses),
      });
    } catch (e) {
      deps.logger.warn("failed to list workspace files", { error: errorMessage(e) });
      return json(res, 400, { error: (e as Error).message });
    }
  }
  if (path === "/api/workspace/file" && req.method === "GET") {
    try {
      const stream = resolveStream(deps, url.searchParams.get("stream"));
      const relativePath = url.searchParams.get("path") ?? "";
      deps.logger.debug("read workspace file", { streamId: stream.id, path: relativePath });
      return json(res, 200, readWorkspaceFile(stream.worktree_path, relativePath));
    } catch (e) {
      deps.logger.warn("failed to read workspace file", { error: errorMessage(e) });
      return json(res, 400, { error: (e as Error).message });
    }
  }
  if (path === "/api/workspace/file" && req.method === "PUT") {
    return readBody(req, (body) => {
      const parsed = parseJsonBody(body);
      if (!parsed || typeof parsed.path !== "string" || typeof parsed.content !== "string") {
        return json(res, 400, { error: "expected { path, content }" });
      }
      try {
        const stream = resolveStream(deps, url.searchParams.get("stream"));
        deps.logger.info("write workspace file", { streamId: stream.id, path: parsed.path });
        const saved = writeWorkspaceFile(stream.worktree_path, parsed.path, parsed.content);
        deps.workspaceWatchers.notify(stream.id, "updated", saved.path);
        return json(res, 200, saved);
      } catch (e) {
        deps.logger.warn("failed to write workspace file", { error: errorMessage(e) });
        return json(res, 400, { error: (e as Error).message });
      }
    });
  }
  if (path === "/api/workspace/watch" && req.method === "GET") {
    const streamId = resolveStreamSelector(url.searchParams.get("stream"), deps.store.getCurrentStreamId() ?? undefined);
    return handleWorkspaceWatchStream(res, deps, streamId);
  }
  if (path === "/api/logs/ui" && req.method === "POST") {
    return readBody(req, (body) => {
      const parsed = parseJsonBody(body);
      if (!parsed || typeof parsed.clientId !== "string" || typeof parsed.message !== "string") {
        deps.logger.warn("rejected invalid ui log payload");
        return json(res, 400, { error: "expected { clientId, message, level?, context?, timestamp? }" });
      }
      const level = parseLogLevel(parsed.level);
      const logger = createUiClientLogger(deps.projectDir, parsed.clientId);
      logger[level](parsed.message, {
        ...(isRecord(parsed.context) ? parsed.context : {}),
        ...(typeof parsed.timestamp === "string" ? { clientTime: parsed.timestamp } : {}),
      });
      return json(res, 200, { ok: true });
    });
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
      deps.logger.debug("ingested hook event", {
        streamId,
        pane,
        hookEvent: event,
        sessionId: stored.normalized.sessionId,
      });
      if (pane && deps.store.get(streamId)) {
        const update = deps.resumeTracker.recordHookEvent(streamId, pane, event, stored.normalized.sessionId);
        if (update?.type === "set") {
          deps.store.update(streamId, (stream) => ({
            ...stream,
            resume: pane === "working"
              ? { ...stream.resume, working_session_id: update.sessionId }
              : { ...stream.resume, talking_session_id: update.sessionId },
          }));
          deps.logger.info("updated resume session id", {
            streamId,
            pane,
            sessionId: update.sessionId,
          });
        } else if (update?.type === "clear") {
          deps.store.update(streamId, (stream) => ({
            ...stream,
            resume: pane === "working"
              ? { ...stream.resume, working_session_id: "" }
              : { ...stream.resume, talking_session_id: "" },
          }));
          deps.logger.warn("cleared stale resume session id", {
            streamId,
            pane,
          });
        }
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
  if (!isGitRepo(deps.projectDir)) {
    return json(res, 400, { error: "git functionality is disabled for this workspace root" });
  }
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
        deps.logger.info("reused existing stream", { streamId: existing.id, branch: localBranch });
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
      deps.logger.info("created stream from existing branch", {
        streamId: stream.id,
        branch: localBranch,
        branchRef: branch.ref,
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
      deps.logger.info("created stream from new branch", {
        streamId: stream.id,
        branch: branchName,
        startPointRef: body.startPointRef,
      });
    } else {
      return json(res, 400, { error: "source must be 'existing' or 'new'" });
    }
    deps.workspaceWatchers.ensureWatching(stream);
    deps.store.setCurrentStreamId(stream.id);
    return json(res, 201, stream);
  } catch (e) {
    deps.logger.warn("failed to create stream", { error: errorMessage(e) });
    return json(res, 400, { error: (e as Error).message });
  }
}

function handleWorkspaceWatchStream(res: ServerResponse, deps: Deps, streamId?: string) {
  res.writeHead(200, {
    "content-type": "text/event-stream",
    "cache-control": "no-cache, no-transform",
    connection: "keep-alive",
    "x-accel-buffering": "no",
  });
  const unsub = deps.workspaceWatchers.subscribe((evt) => {
    res.write(`data: ${JSON.stringify(evt)}\n\n`);
  }, streamId);
  const keepalive = setInterval(() => res.write(": ping\n\n"), 15000);
  res.on("close", () => {
    clearInterval(keepalive);
    unsub();
  });
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

function resolveStream(deps: Deps, streamId: string | null): Stream {
  const id = streamId ?? deps.store.getCurrentStreamId();
  if (!id) throw new Error("no current stream");
  const stream = deps.store.get(id);
  if (!stream) throw new Error(`unknown stream: ${id}`);
  return stream;
}

function parseLogLevel(value: unknown): LogLevel {
  return value === "debug" || value === "info" || value === "warn" || value === "error" ? value : "info";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
