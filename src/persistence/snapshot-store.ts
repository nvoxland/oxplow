import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { join, resolve } from "node:path";
import { createId } from "../core/ids.js";
import type { Logger } from "../core/logger.js";
import { getStateDatabase } from "./state-db.js";

export type SnapshotKind = "turn-start" | "turn-end";

/**
 * `state` discriminates how the entry was captured:
 * - `present`: file exists and was blobbed. `hash` points at a real blob.
 * - `deleted`: file was gone at flush time. No blob; hash is empty.
 * - `oversize`: file existed but exceeded `snapshotMaxFileBytes`. `mtime_ms`
 *    and `size` are recorded so diffs can still say "it changed (by this
 *    much)" even when content isn't available. `hash` is empty.
 */
export type SnapshotEntryState = "present" | "deleted" | "oversize";

export interface SnapshotEntry {
  hash: string;
  mtime_ms: number;
  size: number;
  state: SnapshotEntryState;
}

export interface SnapshotFileRow {
  entry: SnapshotEntry;
  kind: "created" | "updated" | "deleted";
}

export interface SnapshotSummary {
  snapshot: FileSnapshot;
  files: Record<string, SnapshotFileRow>;
  counts: { created: number; updated: number; deleted: number };
}

export interface FileSnapshot {
  id: string;
  stream_id: string;
  worktree_path: string;
  kind: SnapshotKind;
  turn_id: string | null;
  batch_id: string | null;
  parent_snapshot_id: string | null;
  created_at: string;
  turn_prompt: string | null;
}

/**
 * "absent" means the path wasn't found in the chain at all (never tracked).
 * Other values mirror `SnapshotEntryState`.
 */
export type DiffSide = "absent" | SnapshotEntryState;

export interface SnapshotDiffResult {
  before: string | null;
  after: string | null;
  beforeState: DiffSide;
  afterState: DiffSide;
}

export interface FlushInput {
  kind: SnapshotKind;
  streamId: string;
  worktreePath: string;
  dirtyPaths: string[];
  parentSnapshotId: string | null;
  turnId: string | null;
  batchId: string | null;
}

const DEFAULT_MAX_FILE_BYTES = 5 * 1024 * 1024;

export class SnapshotStore {
  private readonly stateDb;
  private readonly rootDir: string;
  private readonly objectsDir: string;
  private maxFileBytes: number = DEFAULT_MAX_FILE_BYTES;
  // LRU-ish cache of resolved entry maps per snapshot so repeat UI calls
  // (summary, diff, reconcile) don't re-walk the parent chain each time.
  // Insertion-order iteration gives us FIFO eviction; cacheGet reinserts
  // to convert that into LRU. Cleared on cleanup.
  private readonly resolveCache = new Map<string, Record<string, SnapshotEntry>>();
  private static readonly RESOLVE_CACHE_MAX = 32;

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.rootDir = join(projectDir, ".newde", "snapshots");
    this.objectsDir = join(this.rootDir, "objects");
    mkdirSync(this.objectsDir, { recursive: true });
  }

  setMaxFileBytes(bytes: number): void {
    // Config validates the realistic floor (>=1024); this setter accepts
    // anything > 0 so tests can exercise the oversize branch with small
    // fixtures. Ignore non-positive values to avoid disabling blobbing
    // entirely by accident.
    if (bytes > 0) this.maxFileBytes = Math.floor(bytes);
  }

  getMaxFileBytes(): number {
    return this.maxFileBytes;
  }

  writeBlob(content: Buffer): string {
    const hash = createHash("sha256").update(content).digest("hex");
    const dir = join(this.objectsDir, hash.slice(0, 2));
    const file = join(dir, hash.slice(2));
    if (!existsSync(file)) {
      mkdirSync(dir, { recursive: true });
      writeFileSync(file, content);
    }
    return hash;
  }

  readBlob(hash: string): Buffer {
    const file = join(this.objectsDir, hash.slice(0, 2), hash.slice(2));
    return readFileSync(file);
  }

  hasBlob(hash: string): boolean {
    return existsSync(join(this.objectsDir, hash.slice(0, 2), hash.slice(2)));
  }

  flushSnapshot(input: FlushInput): string | null {
    if (input.dirtyPaths.length === 0) return null;
    const id = createId("snap");
    const now = new Date().toISOString();
    const unique = Array.from(new Set(input.dirtyPaths)).sort();
    const entries: Array<[string, SnapshotEntry]> = [];

    for (const relpath of unique) {
      const abs = resolve(input.worktreePath, relpath);
      if (!existsSync(abs)) {
        entries.push([relpath, { hash: "", mtime_ms: 0, size: 0, state: "deleted" }]);
        continue;
      }
      let st;
      try {
        st = statSync(abs);
      } catch {
        continue;
      }
      if (!st.isFile()) continue;
      if (st.size > this.maxFileBytes) {
        entries.push([
          relpath,
          { hash: "", mtime_ms: Math.floor(st.mtimeMs), size: st.size, state: "oversize" },
        ]);
        continue;
      }
      const content = readFileSync(abs);
      const hash = this.writeBlob(content);
      entries.push([
        relpath,
        { hash, mtime_ms: Math.floor(st.mtimeMs), size: st.size, state: "present" },
      ]);
    }

    if (entries.length === 0) return null;

    this.stateDb.transaction(() => {
      this.stateDb.run(
        `INSERT INTO file_snapshot (
          id, stream_id, worktree_path, kind, turn_id, batch_id,
          parent_snapshot_id, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
        id,
        input.streamId,
        input.worktreePath,
        input.kind,
        input.turnId,
        input.batchId,
        input.parentSnapshotId,
        now,
      );
      for (const [path, entry] of entries) {
        this.stateDb.run(
          `INSERT INTO snapshot_entry (snapshot_id, path, hash, mtime_ms, size, state)
           VALUES (?, ?, ?, ?, ?, ?)`,
          id,
          path,
          entry.hash,
          entry.mtime_ms,
          entry.size,
          entry.state,
        );
      }
    });
    return id;
  }

  getSnapshot(id: string): FileSnapshot | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      "SELECT * FROM file_snapshot WHERE id = ? LIMIT 1",
      id,
    );
    return row ? rowToSnapshot(row) : null;
  }

  loadManifestEntries(id: string): Record<string, SnapshotEntry> {
    const rows = this.stateDb.all<Record<string, unknown>>(
      "SELECT path, hash, mtime_ms, size, state FROM snapshot_entry WHERE snapshot_id = ?",
      id,
    );
    const out: Record<string, SnapshotEntry> = {};
    for (const row of rows) {
      out[String(row.path)] = rowToEntry(row);
    }
    return out;
  }

  /** Merge entries walking the parent chain (newest wins). Tombstones kept. */
  resolveEntries(id: string): Record<string, SnapshotEntry> {
    const cached = this.cacheGet(id);
    if (cached) return cached;
    const merged: Record<string, SnapshotEntry> = {};
    const visited = new Set<string>();
    const chain: string[] = [];
    let cursor: string | null = id;
    while (cursor) {
      if (visited.has(cursor)) {
        // Defensive: broken chain (shouldn't happen given FK semantics).
        // Stop walking rather than loop forever.
        this.logger?.warn("snapshot parent chain cycle detected", { snapshotId: cursor });
        break;
      }
      visited.add(cursor);
      chain.push(cursor);
      const snap = this.getSnapshot(cursor);
      cursor = snap?.parent_snapshot_id ?? null;
    }
    // Walk oldest → newest so newer entries overwrite older ones.
    for (let i = chain.length - 1; i >= 0; i--) {
      const entries = this.loadManifestEntries(chain[i]!);
      for (const [path, entry] of Object.entries(entries)) {
        merged[path] = entry;
      }
    }
    this.cacheSet(id, merged);
    return merged;
  }

  private cacheGet(id: string): Record<string, SnapshotEntry> | undefined {
    const hit = this.resolveCache.get(id);
    if (hit) {
      // Re-insert to move the key to the end (most-recent) of the
      // insertion-order Map — cheap LRU.
      this.resolveCache.delete(id);
      this.resolveCache.set(id, hit);
    }
    return hit;
  }

  private cacheSet(id: string, value: Record<string, SnapshotEntry>): void {
    if (this.resolveCache.size >= SnapshotStore.RESOLVE_CACHE_MAX) {
      const first = this.resolveCache.keys().next().value;
      if (first !== undefined) this.resolveCache.delete(first);
    }
    this.resolveCache.set(id, value);
  }

  /** Resolved hash for `relpath` at snapshot `id`, or null if deleted/absent. */
  resolvePath(id: string, relpath: string): string | null {
    const entry = this.resolveEntries(id)[relpath];
    if (!entry || entry.state !== "present" || !entry.hash) return null;
    return entry.hash;
  }

  /** Resolved entry (with state) for `relpath` at `id`, or null if absent. */
  resolveEntry(id: string, relpath: string): SnapshotEntry | null {
    return this.resolveEntries(id)[relpath] ?? null;
  }

  diffPath(beforeId: string | null, afterId: string, relpath: string): SnapshotDiffResult {
    const beforeSide = this.resolveDiffSide(beforeId, relpath);
    const afterSide = this.resolveDiffSide(afterId, relpath);
    return {
      before: beforeSide.content,
      after: afterSide.content,
      beforeState: beforeSide.state,
      afterState: afterSide.state,
    };
  }

  private resolveDiffSide(
    id: string | null,
    relpath: string,
  ): { content: string | null; state: DiffSide } {
    if (!id) return { content: null, state: "absent" };
    const entry = this.resolveEntry(id, relpath);
    if (!entry) return { content: null, state: "absent" };
    if (entry.state === "present") {
      return { content: this.readBlobAsText(entry.hash), state: "present" };
    }
    // deleted / oversize: no content to show, but signal the state so the UI
    // can render an explanation rather than a blank pane.
    return { content: null, state: entry.state };
  }

  private readBlobAsText(hash: string): string | null {
    if (!hash) return null;
    try {
      return this.readBlob(hash).toString("utf8");
    } catch {
      return null;
    }
  }

  /**
   * Summary for the UI: row + per-file entries + A/M/D classification.
   * Oversize entries stay in `files` (visible) but they don't participate
   * in the add/update/delete counters — there's no real content churn to
   * report, just a stat delta.
   */
  getSnapshotSummary(snapshotId: string): SnapshotSummary | null {
    const snap = this.getSnapshot(snapshotId);
    if (!snap) return null;
    const entries = this.loadManifestEntries(snapshotId);
    const parentEntries = snap.parent_snapshot_id
      ? this.resolveEntries(snap.parent_snapshot_id)
      : {};
    const files: Record<string, SnapshotFileRow> = {};
    let created = 0;
    let updated = 0;
    let deleted = 0;
    for (const [path, entry] of Object.entries(entries)) {
      let kind: "created" | "updated" | "deleted";
      if (entry.state === "deleted") {
        kind = "deleted";
        deleted++;
      } else if (!parentEntries[path] || parentEntries[path].state === "deleted") {
        kind = "created";
        if (entry.state !== "oversize") created++;
      } else {
        kind = "updated";
        const parent = parentEntries[path];
        // Only count as "updated" if something actually changed. For present
        // entries that's hash inequality; for oversize it's mtime or size.
        const parentIsPresent = parent.state === "present";
        const entryIsPresent = entry.state === "present";
        const hashChanged = entryIsPresent && parentIsPresent && parent.hash !== entry.hash;
        const statChanged = parent.mtime_ms !== entry.mtime_ms || parent.size !== entry.size;
        const stateChanged = parent.state !== entry.state;
        if (hashChanged || statChanged || stateChanged) updated++;
      }
      files[path] = { entry, kind };
    }
    return {
      snapshot: snap,
      files,
      counts: { created, updated, deleted },
    };
  }

  getSnapshotFileDiff(snapshotId: string, path: string): SnapshotDiffResult {
    const snap = this.getSnapshot(snapshotId);
    if (!snap) {
      return { before: null, after: null, beforeState: "absent", afterState: "absent" };
    }
    return this.diffPath(snap.parent_snapshot_id ?? null, snapshotId, path);
  }

  getSnapshotPairDiff(
    beforeSnapshotId: string | null,
    afterSnapshotId: string,
    path: string,
  ): SnapshotDiffResult {
    return this.diffPath(beforeSnapshotId, afterSnapshotId, path);
  }

  /**
   * Walk the worktree and compare (mtime_ms, size) against the resolved
   * entry map for `snapshotId`. Returned paths are the ones whose stat
   * differs, or that exist on disk but not in the manifest, or vice versa.
   * Oversize-on-disk files are surfaced too — they get a stat-only entry
   * when next flushed.
   */
  reconcileWorktree(
    snapshotId: string,
    worktreePath: string,
    ignore: (relpath: string) => boolean,
  ): string[] {
    const entries = this.resolveEntries(snapshotId);
    const dirty = new Set<string>();
    const seen = new Set<string>();

    walkAll(worktreePath, "", ignore, (rel, st) => {
      seen.add(rel);
      const entry = entries[rel];
      const size = st.size;
      const mtime = Math.floor(st.mtimeMs);
      if (!entry || entry.state === "deleted") {
        dirty.add(rel);
        return;
      }
      if (entry.size !== size || entry.mtime_ms !== mtime) {
        dirty.add(rel);
      }
    });

    for (const [rel, entry] of Object.entries(entries)) {
      if (entry.state === "deleted") continue;
      if (!seen.has(rel)) dirty.add(rel);
    }
    return Array.from(dirty).sort();
  }

  /**
   * Drop snapshots older than `retentionDays` while preserving each
   * stream's most recent snapshot (and anything `streams.current_snapshot_id`
   * points at) no matter its age. After deleting snapshot rows (cascades
   * remove their `snapshot_entry` rows), sweep `objects/` for blobs that
   * are no longer referenced. `retentionDays === 0` disables pruning.
   */
  cleanupOldSnapshots(retentionDays: number): { snapshotsDeleted: number; blobsDeleted: number } {
    if (retentionDays <= 0) return { snapshotsDeleted: 0, blobsDeleted: 0 };
    const cutoff = new Date(Date.now() - retentionDays * 86400_000).toISOString();
    const keep = new Set<string>();

    const latestRows = this.stateDb.all<{ id: string }>(
      `SELECT id FROM file_snapshot f1
       WHERE created_at = (
         SELECT MAX(created_at) FROM file_snapshot f2 WHERE f2.stream_id = f1.stream_id
       )`,
    );
    for (const row of latestRows) keep.add(row.id);

    const pointedRows = this.stateDb.all<{ current_snapshot_id: string | null }>(
      `SELECT current_snapshot_id FROM streams WHERE current_snapshot_id IS NOT NULL`,
    );
    for (const row of pointedRows) {
      if (row.current_snapshot_id) keep.add(row.current_snapshot_id);
    }

    const staleRows = this.stateDb.all<{ id: string }>(
      `SELECT id FROM file_snapshot WHERE created_at < ?`,
      cutoff,
    );
    const toDelete = staleRows.filter((row) => !keep.has(row.id));
    if (toDelete.length === 0) return { snapshotsDeleted: 0, blobsDeleted: 0 };

    this.stateDb.transaction(() => {
      for (const row of toDelete) {
        this.stateDb.run("DELETE FROM file_snapshot WHERE id = ?", row.id);
      }
    });
    this.resolveCache.clear();

    const blobsDeleted = this.gcBlobs();
    return { snapshotsDeleted: toDelete.length, blobsDeleted };
  }

  /**
   * Sweep `objects/` for blobs whose sha is not referenced by any
   * surviving `snapshot_entry` row. Returns the count removed.
   */
  gcBlobs(): number {
    const referenced = new Set<string>();
    const rows = this.stateDb.all<{ hash: string }>(
      "SELECT DISTINCT hash FROM snapshot_entry WHERE state = 'present' AND hash != ''",
    );
    for (const row of rows) referenced.add(row.hash);

    let removed = 0;
    let shardDirs;
    try {
      shardDirs = readdirSync(this.objectsDir, { withFileTypes: true });
    } catch {
      return 0;
    }
    for (const shard of shardDirs) {
      if (!shard.isDirectory()) continue;
      const shardPath = join(this.objectsDir, shard.name);
      let entries;
      try {
        entries = readdirSync(shardPath, { withFileTypes: true });
      } catch {
        continue;
      }
      for (const entry of entries) {
        if (!entry.isFile()) continue;
        const hash = `${shard.name}${entry.name}`;
        if (referenced.has(hash)) continue;
        try {
          unlinkSync(join(shardPath, entry.name));
          removed++;
        } catch (error) {
          this.logger?.warn("snapshot blob unlink failed", {
            hash,
            error: error instanceof Error ? error.message : String(error),
          });
        }
      }
    }
    return removed;
  }

  listSnapshotsForStream(streamId: string, limit = 100): FileSnapshot[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT fs.*, at.prompt AS turn_prompt
         FROM file_snapshot fs
         LEFT JOIN agent_turn at ON fs.turn_id = at.id
         WHERE fs.stream_id = ?
         ORDER BY fs.created_at DESC, fs.rowid DESC
         LIMIT ?`,
        streamId,
        limit,
      )
      .map(rowToSnapshotWithPrompt);
  }

  getTurnSnapshots(turnId: string): { start: FileSnapshot | null; end: FileSnapshot | null } {
    const rows = this.stateDb.all<Record<string, unknown>>(
      "SELECT * FROM file_snapshot WHERE turn_id = ? ORDER BY created_at ASC",
      turnId,
    );
    const snaps = rows.map(rowToSnapshot);
    return {
      start: snaps.find((s) => s.kind === "turn-start") ?? null,
      end: snaps.find((s) => s.kind === "turn-end") ?? null,
    };
  }
}

function rowToSnapshot(row: Record<string, unknown>): FileSnapshot {
  return {
    id: String(row.id ?? ""),
    stream_id: String(row.stream_id ?? ""),
    worktree_path: String(row.worktree_path ?? ""),
    kind: String(row.kind ?? "turn-end") as SnapshotKind,
    turn_id: row.turn_id == null ? null : String(row.turn_id),
    batch_id: row.batch_id == null ? null : String(row.batch_id),
    parent_snapshot_id: row.parent_snapshot_id == null ? null : String(row.parent_snapshot_id),
    created_at: String(row.created_at ?? ""),
    turn_prompt: null,
  };
}

function rowToSnapshotWithPrompt(row: Record<string, unknown>): FileSnapshot {
  return {
    ...rowToSnapshot(row),
    turn_prompt: row.turn_prompt == null ? null : String(row.turn_prompt),
  };
}

function rowToEntry(row: Record<string, unknown>): SnapshotEntry {
  const state = String(row.state ?? "present") as SnapshotEntryState;
  return {
    hash: String(row.hash ?? ""),
    mtime_ms: Number(row.mtime_ms ?? 0),
    size: Number(row.size ?? 0),
    state,
  };
}

/**
 * Walks every file under `root`, calling `onFile` for each one — including
 * files that would be too big to blob. The callback receives the stat so the
 * caller can decide how to classify (present vs oversize). Ignore function
 * is consulted per path segment.
 */
function walkAll(
  root: string,
  rel: string,
  ignore: (relpath: string) => boolean,
  onFile: (relpath: string, st: import("node:fs").Stats) => void,
): void {
  const abs = rel ? join(root, rel) : root;
  let entries;
  try {
    entries = readdirSync(abs, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    const childRel = rel ? `${rel}/${entry.name}` : entry.name;
    if (ignore(childRel)) continue;
    if (entry.isDirectory()) {
      walkAll(root, childRel, ignore, onFile);
      continue;
    }
    if (!entry.isFile()) continue;
    let st;
    try {
      st = statSync(join(root, childRel));
    } catch {
      continue;
    }
    onFile(childRel, st);
  }
}
