import { existsSync, statSync, watch, type FSWatcher } from "node:fs";
import { join, resolve } from "node:path";
import type { Logger } from "../core/logger.js";
import type { Stream } from "../persistence/stream-store.js";

export interface GitRefsChange {
  streamId: string;
  t: number;
}

/**
 * Watches each stream's `.git` directory and emits a debounced change event
 * on commits, rollbacks, pulls, pushes, checkouts, or any other git operation
 * that mutates refs/HEAD/index. The workspace watcher ignores `.git` by
 * design so we can't piggyback on it.
 *
 * Debouncing: a single git command can fire a dozen fs events (HEAD, refs/*,
 * logs/*, index, ORIG_HEAD, …). We collapse bursts into one emission ~200 ms
 * after the last event so downstream subscribers refresh once per operation.
 */
export class GitRefsWatcherRegistry {
  private readonly watchers = new Map<string, StreamGitRefsWatcher>();
  private readonly subscribers = new Set<(change: GitRefsChange) => void>();

  constructor(private readonly logger: Logger) {}

  ensureWatching(stream: Stream): void {
    const existing = this.watchers.get(stream.id);
    const rootDir = resolve(stream.worktree_path);
    if (existing && existing.rootDir === rootDir) return;
    existing?.dispose();
    const watcher = new StreamGitRefsWatcher(
      stream,
      () => this.emit(stream.id),
      this.logger.child({ streamId: stream.id, subsystem: "git-refs-watch" }),
    );
    watcher.start();
    this.watchers.set(stream.id, watcher);
  }

  stopWatching(streamId: string): void {
    const existing = this.watchers.get(streamId);
    if (!existing) return;
    existing.dispose();
    this.watchers.delete(streamId);
  }

  subscribe(fn: (change: GitRefsChange) => void): () => void {
    this.subscribers.add(fn);
    return () => this.subscribers.delete(fn);
  }

  dispose(): void {
    for (const watcher of this.watchers.values()) watcher.dispose();
    this.watchers.clear();
    this.subscribers.clear();
  }

  private emit(streamId: string): void {
    const change: GitRefsChange = { streamId, t: Date.now() };
    for (const fn of this.subscribers) {
      try { fn(change); } catch {}
    }
  }
}

const DEBOUNCE_MS = 200;

class StreamGitRefsWatcher {
  readonly rootDir: string;
  private readonly watchers = new Map<string, FSWatcher>();
  private debounceTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    stream: Stream,
    private readonly emit: () => void,
    private readonly logger: Logger,
  ) {
    this.rootDir = resolve(stream.worktree_path);
  }

  start(): void {
    const gitDir = this.resolveGitDir();
    if (!gitDir) {
      // Not a git checkout (or `.git` is a worktree pointer we can't read).
      // The runtime's gitRootWatcher will (re)start us if a repo appears.
      return;
    }
    if (this.tryRecursive(gitDir)) return;
    // Fall back to watching the top-level .git entries that change on common
    // git operations — enough to catch commits/checkouts/pulls without the
    // cost of per-dir watchers across refs/ and logs/.
    this.watchDir(gitDir);
    for (const sub of ["refs", join("refs", "heads"), join("refs", "remotes"), join("refs", "tags"), "logs", "logs/refs"]) {
      const path = join(gitDir, sub);
      if (existsSync(path) && safeStat(path)?.isDirectory()) this.watchDir(path);
    }
  }

  dispose(): void {
    for (const watcher of this.watchers.values()) watcher.close();
    this.watchers.clear();
    if (this.debounceTimer) clearTimeout(this.debounceTimer);
    this.debounceTimer = null;
  }

  /** Resolves `.git`: a directory in the main checkout, a pointer file in a
   *  worktree. We don't follow pointer files here — secondary worktrees are
   *  refused at startup, and the runtime is the only consumer of this. */
  private resolveGitDir(): string | null {
    const dotGit = join(this.rootDir, ".git");
    const stat = safeStat(dotGit);
    if (!stat) return null;
    if (stat.isDirectory()) return dotGit;
    return null;
  }

  private tryRecursive(gitDir: string): boolean {
    try {
      const watcher = watch(gitDir, { recursive: true }, () => this.schedule());
      watcher.on("error", (error) => {
        this.logger.warn("git-refs watcher error", { error: errorMessage(error) });
      });
      this.watchers.set(gitDir, watcher);
      return true;
    } catch (error) {
      this.logger.info("recursive fs.watch unavailable for .git, falling back", {
        error: errorMessage(error),
      });
      return false;
    }
  }

  private watchDir(dir: string): void {
    if (this.watchers.has(dir)) return;
    try {
      const watcher = watch(dir, () => this.schedule());
      watcher.on("error", (error) => {
        this.logger.warn("git-refs watcher error", { dir, error: errorMessage(error) });
      });
      this.watchers.set(dir, watcher);
    } catch (error) {
      this.logger.warn("failed to watch .git subdir", { dir, error: errorMessage(error) });
    }
  }

  private schedule(): void {
    if (this.debounceTimer) clearTimeout(this.debounceTimer);
    this.debounceTimer = setTimeout(() => {
      this.debounceTimer = null;
      this.emit();
    }, DEBOUNCE_MS);
  }
}

function safeStat(path: string) {
  try { return statSync(path); } catch { return null; }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
