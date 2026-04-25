import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  watch,
  type FSWatcher,
} from "node:fs";
import { join } from "node:path";
import type { Logger } from "../core/logger.js";
import { parseNoteRefs } from "../persistence/wiki-note-refs.js";
import type { NoteRefSnapshot, WikiNoteStore } from "../persistence/wiki-note-store.js";
import { readWorktreeHeadSha } from "./git.js";

const NOTES_SUBDIR = join(".oxplow", "notes");
const MD_SUFFIX = ".md";

export function notesDir(projectDir: string): string {
  return join(projectDir, NOTES_SUBDIR);
}

/**
 * Re-parse a single note file and upsert (or delete) its row. Called by the
 * watcher after debouncing and on initial scan. Body lives on disk; this
 * derives the metadata fields (title, references, HEAD sha, per-ref blob
 * hashes) from the current file contents + workspace state.
 */
export function syncNoteFromDisk(
  projectDir: string,
  store: WikiNoteStore,
  slug: string,
): void {
  const filePath = join(notesDir(projectDir), `${slug}${MD_SUFFIX}`);
  if (!existsSync(filePath)) {
    store.deleteBySlug(slug);
    return;
  }
  const body = readFileSync(filePath, "utf8");
  const title = extractTitle(body, slug);
  const refs = parseNoteRefs(body);
  const capturedHeadSha = readWorktreeHeadSha(projectDir);
  const capturedRefs: NoteRefSnapshot[] = refs.map((r) => {
    const abs = join(projectDir, r.path);
    return { path: r.path, blobSha: hashFile(abs), mtimeMs: mtimeMsOf(abs) };
  });
  store.upsert({ slug, title, body, capturedHeadSha, capturedRefs });
}

/**
 * Sync every `.md` file in the notes directory and prune rows that no
 * longer have a matching file. Called once at watcher startup.
 */
export function scanAndSyncAll(projectDir: string, store: WikiNoteStore): void {
  const dir = notesDir(projectDir);
  mkdirSync(dir, { recursive: true });
  const present = new Set<string>();
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (!entry.isFile() || !entry.name.endsWith(MD_SUFFIX)) continue;
    const slug = entry.name.slice(0, -MD_SUFFIX.length);
    present.add(slug);
    syncNoteFromDisk(projectDir, store, slug);
  }
  for (const row of store.list()) {
    if (!present.has(row.slug)) {
      store.deleteBySlug(row.slug);
    }
  }
}

export interface NotesWatcherOptions {
  debounceMs?: number;
}

export class NotesWatcher {
  private watcher: FSWatcher | null = null;
  private readonly pending = new Map<string, NodeJS.Timeout>();
  private readonly debounceMs: number;

  constructor(
    private readonly projectDir: string,
    private readonly store: WikiNoteStore,
    options: NotesWatcherOptions = {},
    private readonly logger?: Logger,
  ) {
    this.debounceMs = options.debounceMs ?? 200;
  }

  start(): void {
    const dir = notesDir(this.projectDir);
    mkdirSync(dir, { recursive: true });
    scanAndSyncAll(this.projectDir, this.store);
    try {
      this.watcher = watch(dir, (_event, filename) => {
        if (typeof filename !== "string" || filename.length === 0) return;
        this.schedule(filename);
      });
      this.watcher.on("error", (error) => {
        this.logger?.warn("notes watcher error", { error: errorMessage(error) });
      });
    } catch (error) {
      this.logger?.warn("notes watcher failed to start", { error: errorMessage(error) });
    }
  }

  dispose(): void {
    this.watcher?.close();
    this.watcher = null;
    for (const timer of this.pending.values()) clearTimeout(timer);
    this.pending.clear();
  }

  private schedule(filename: string): void {
    if (!filename.endsWith(MD_SUFFIX)) return;
    const slug = filename.slice(0, -MD_SUFFIX.length);
    if (!slug) return;
    const prev = this.pending.get(slug);
    if (prev) clearTimeout(prev);
    this.pending.set(
      slug,
      setTimeout(() => {
        this.pending.delete(slug);
        try {
          syncNoteFromDisk(this.projectDir, this.store, slug);
        } catch (error) {
          this.logger?.warn("notes sync failed", { slug, error: errorMessage(error) });
        }
      }, this.debounceMs),
    );
  }
}

function extractTitle(body: string, fallback: string): string {
  const m = /^#\s+(.+)$/m.exec(body);
  const title = m?.[1]?.trim();
  return title && title.length > 0 ? title : fallback;
}

function hashFile(abs: string): string | null {
  try {
    const buf = readFileSync(abs);
    return createHash("sha256").update(buf).digest("hex");
  } catch {
    return null;
  }
}

/** Exposed so the MCP / freshness layer can recompute blob hashes without
 *  going through the watcher. Returns null if the file is missing. */
export function hashWorkspaceFile(projectDir: string, relPath: string): string | null {
  return hashFile(join(projectDir, relPath));
}

function mtimeMsOf(abs: string): number | null {
  try {
    return statSync(abs).mtimeMs;
  } catch {
    return null;
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
