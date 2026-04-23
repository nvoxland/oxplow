import { existsSync, readdirSync, statSync, watch, type FSWatcher } from "node:fs";
import { relative, resolve, sep } from "node:path";
import type { Logger } from "../core/logger.js";
import type { Stream } from "../persistence/stream-store.js";

export type WorkspaceWatchKind = "created" | "updated" | "deleted";

export interface WorkspaceWatchEvent {
  id: number;
  streamId: string;
  path: string;
  kind: WorkspaceWatchKind;
  t: number;
}

export class WorkspaceWatcherRegistry {
  private readonly watchers = new Map<string, StreamWorkspaceWatcher>();
  private readonly subscribers = new Set<(event: WorkspaceWatchEvent) => void>();
  private nextId = 1;
  private extraIgnoreDirs: string[] = [];

  constructor(private readonly logger: Logger) {}

  setExtraIgnoreDirs(dirs: string[]): void {
    // Stored by reference so the filter inside each watcher sees live
    // updates when the user edits `generatedDirs` in oxplow.yaml. Callers
    // that mutate the config should call this again on reload.
    this.extraIgnoreDirs = [...dirs];
  }

  ensureWatching(stream: Stream): void {
    const existing = this.watchers.get(stream.id);
    if (existing && existing.rootDir === resolve(stream.worktree_path)) {
      return;
    }
    existing?.dispose();
    const watcher = new StreamWorkspaceWatcher(
      stream,
      (kind, path) => this.emit(stream.id, kind, path),
      this.logger.child({ streamId: stream.id, subsystem: "workspace-watch" }),
      () => this.extraIgnoreDirs,
    );
    watcher.start();
    this.watchers.set(stream.id, watcher);
  }

  notify(streamId: string, kind: WorkspaceWatchKind, path: string): WorkspaceWatchEvent {
    return this.emit(streamId, kind, path);
  }

  subscribe(fn: (event: WorkspaceWatchEvent) => void, streamId?: string): () => void {
    const wrapped = (event: WorkspaceWatchEvent) => {
      if (streamId && event.streamId !== streamId) return;
      fn(event);
    };
    this.subscribers.add(wrapped);
    return () => this.subscribers.delete(wrapped);
  }

  dispose(): void {
    for (const watcher of this.watchers.values()) {
      watcher.dispose();
    }
    this.watchers.clear();
    this.subscribers.clear();
  }

  private emit(streamId: string, kind: WorkspaceWatchKind, path: string): WorkspaceWatchEvent {
    const event: WorkspaceWatchEvent = {
      id: this.nextId++,
      streamId,
      kind,
      path,
      t: Date.now(),
    };
    for (const subscriber of this.subscribers) {
      try {
        subscriber(event);
      } catch {}
    }
    return event;
  }
}

class StreamWorkspaceWatcher {
  readonly rootDir: string;
  private readonly watchers = new Map<string, FSWatcher>();
  private recursive = false;

  constructor(
    stream: Stream,
    private readonly emit: (kind: WorkspaceWatchKind, path: string) => void,
    private readonly logger: Logger,
    private readonly getExtraIgnoreDirs: () => string[] = () => [],
  ) {
    this.rootDir = resolve(stream.worktree_path);
  }

  private shouldIgnore(path: string): boolean {
    return shouldIgnoreWorkspaceWatchPath(path, this.getExtraIgnoreDirs());
  }

  start(): void {
    if (!existsSync(this.rootDir) || !safeStat(this.rootDir)?.isDirectory()) {
      this.logger.warn("workspace root missing for watcher", { rootDir: this.rootDir });
      return;
    }
    if (this.tryRecursive()) return;
    this.watchDirectory(this.rootDir);
  }

  dispose(): void {
    for (const watcher of this.watchers.values()) {
      watcher.close();
    }
    this.watchers.clear();
  }

  private tryRecursive(): boolean {
    try {
      const watcher = watch(this.rootDir, { recursive: true }, (eventType, filename) => {
        const name = typeof filename === "string"
          ? filename
          : filename != null
            ? (filename as Buffer).toString("utf8")
            : "";
        this.handleRecursiveEvent(eventType, name);
      });
      watcher.on("error", (error) => {
        this.logger.warn("workspace watcher error", { error: errorMessage(error) });
      });
      this.watchers.set(this.rootDir, watcher);
      this.recursive = true;
      return true;
    } catch (error) {
      this.logger.info("recursive fs.watch unavailable, falling back to per-directory", {
        error: errorMessage(error),
      });
      return false;
    }
  }

  private handleRecursiveEvent(eventType: string, filename: string): void {
    if (!filename) return;
    const rel = normalizeRelativePath(filename);
    if (!rel || rel.startsWith("..")) return;
    if (this.shouldIgnore(rel)) return;
    const abs = resolve(this.rootDir, rel);
    const stat = safeStat(abs);
    if (eventType === "change") {
      this.emit("updated", rel);
      return;
    }
    this.emit(stat ? "created" : "deleted", rel);
  }

  private watchDirectory(dir: string): void {
    if (this.watchers.has(dir)) return;
    const dirRelativePath = normalizeRelativePath(relative(this.rootDir, dir));
    if (dirRelativePath && this.shouldIgnore(dirRelativePath)) {
      return;
    }
    const watcher = watch(dir, (eventType, filename) => {
      const name = typeof filename === "string"
        ? filename
        : filename != null
          ? (filename as Buffer).toString("utf8")
          : "";
      this.handleFsEvent(dir, eventType, name);
    });
    watcher.on("error", (error) => {
      this.logger.warn("workspace watcher error", { dir, error: errorMessage(error) });
    });
    this.watchers.set(dir, watcher);

    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (entry.isDirectory()) {
        this.watchDirectory(resolve(dir, entry.name));
      }
    }
  }

  private handleFsEvent(dir: string, eventType: string, filename: string): void {
    if (!filename) {
      return;
    }
    const abs = resolve(dir, filename);
    const rel = normalizeRelativePath(relative(this.rootDir, abs));
    if (rel.startsWith("..")) return;
    if (this.shouldIgnore(rel)) return;

    const stat = safeStat(abs);
    if (stat?.isDirectory()) {
      this.watchDirectory(abs);
    } else if (!stat) {
      this.unwatchDescendants(abs);
    }

    if (eventType === "change") {
      this.emit("updated", rel);
      return;
    }
    this.emit(stat ? "created" : "deleted", rel);
  }

  private unwatchDescendants(absPath: string): void {
    if (this.recursive) return;
    for (const [dir, watcher] of this.watchers) {
      if (dir === absPath || dir.startsWith(absPath + sep)) {
        watcher.close();
        this.watchers.delete(dir);
      }
    }
  }
}

const IGNORED_DIR_NAMES = new Set([
  ".git",
  "node_modules",
  "dist",
  "build",
  "out",
  "target",
  "coverage",
  "__pycache__",
  ".venv",
  ".next",
  ".nuxt",
  ".svelte-kit",
  ".turbo",
  ".cache",
  ".parcel-cache",
  ".idea",
  ".vscode",
  ".gradle",
  ".pytest_cache",
  ".mypy_cache",
  ".tox",
]);

export function isTempFilename(name: string): boolean {
  if (!name) return false;
  // Patterns like `foo.md.tmp.18726.1776752930633`
  if (/\.tmp\.\d+\.\d+$/.test(name)) return true;
  if (name.endsWith(".tmp")) return true;
  if (name.endsWith("~")) return true;
  if (name.endsWith(".swp") || name.endsWith(".swx") || name.endsWith(".swo")) return true;
  // Emacs lock/autosave: `#foo#`
  if (name.startsWith("#") && name.endsWith("#") && name.length >= 2) return true;
  return false;
}

export function shouldIgnoreWorkspaceWatchPath(path: string, extraIgnoreDirs: string[] = []): boolean {
  if (path === ".oxplow" || path.startsWith(".oxplow/")) return true;
  const extras = extraIgnoreDirs.length > 0 ? new Set(extraIgnoreDirs) : null;
  const segments = path.split("/");
  for (const segment of segments) {
    if (IGNORED_DIR_NAMES.has(segment)) return true;
    if (extras && extras.has(segment)) return true;
  }
  const basename = segments[segments.length - 1] ?? "";
  if (isTempFilename(basename)) return true;
  return false;
}

function safeStat(path: string) {
  try {
    return statSync(path);
  } catch {
    return null;
  }
}

function normalizeRelativePath(path: string): string {
  if (!path || path === ".") return "";
  return path.replace(/\\/g, "/");
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
