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

export type SnapshotSource =
  | "task-start"
  | "task-end"
  | "turn-start"
  | "turn-end"
  | "startup";

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
  previousSnapshotId: string | null;
  files: Record<string, SnapshotFileRow>;
  counts: { created: number; updated: number; deleted: number };
}

export interface FileSnapshot {
  id: string;
  stream_id: string;
  worktree_path: string;
  version_hash: string;
  source: SnapshotSource;
  created_at: string;
  /**
   * Populated by `listSnapshotsForStream`: the human-readable label for
   * the row (task title + phase when an effort links here; prompt when
   * a turn links here; otherwise null and the UI falls back to `source`).
   */
  label?: string | null;
  /** `"task" | "turn" | "system"` — drives the icon choice in the UI. */
  label_kind?: "task" | "turn" | "system" | null;
}

/**
 * "absent" means the path wasn't found at that snapshot. Other values mirror
 * `SnapshotEntryState`.
 */
export type DiffSide = "absent" | SnapshotEntryState;

export interface SnapshotDiffResult {
  before: string | null;
  after: string | null;
  beforeState: DiffSide;
  afterState: DiffSide;
}

/**
 * Inputs to `flushSnapshot`. `dirtyPaths` is an optimizer hint: when provided,
 * only those paths are re-scanned (their entries are copied from the previous
 * snapshot otherwise). When null/undefined the full worktree is walked.
 */
export interface FlushInput {
  source: SnapshotSource;
  streamId: string;
  worktreePath: string;
  /** Optional — when set, only these relative paths are re-scanned and all
   *  other entries are carried forward from the previous snapshot. */
  dirtyPaths?: string[] | null;
  /** Ignore filter, consulted during full walks. */
  ignore?: (relpath: string) => boolean;
}

export interface FlushResult {
  id: string;
  created: boolean;
  versionHash: string;
}

const DEFAULT_MAX_FILE_BYTES = 5 * 1024 * 1024;

export class SnapshotStore {
  private readonly stateDb;
  private readonly rootDir: string;
  private readonly objectsDir: string;
  private maxFileBytes: number = DEFAULT_MAX_FILE_BYTES;

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.rootDir = join(projectDir, ".newde", "snapshots");
    this.objectsDir = join(this.rootDir, "objects");
    mkdirSync(this.objectsDir, { recursive: true });
  }

  setMaxFileBytes(bytes: number): void {
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

  /**
   * Capture a new snapshot of the stream's worktree. If `dirtyPaths` is
   * provided, only those are re-scanned and other entries carry forward from
   * the previous snapshot; otherwise the entire worktree is walked.
   *
   * Computes a `version_hash` over the final entry set; if it matches the
   * newest existing snapshot for this stream, the existing id is returned
   * with `created: false` (no new row).
   */
  flushSnapshot(input: FlushInput): FlushResult {
    const latest = this.getLatestSnapshot(input.streamId);
    const baselineEntries = latest ? this.loadManifestEntries(latest.id) : {};
    const ignore = input.ignore ?? (() => false);
    const entries = this.buildEntries(input, baselineEntries, ignore);
    const versionHash = computeVersionHash(entries);

    if (latest && latest.version_hash === versionHash) {
      return { id: latest.id, created: false, versionHash };
    }

    const newId = createId("snap");
    const now = new Date().toISOString();
    let resultId = newId;
    let created = true;
    // Re-check `latest` *inside* the transaction and skip the insert if
    // another flush landed a matching-hash row after our initial read. The
    // pre-transaction check above stays as a fast path; this one closes the
    // read-then-write window against a concurrent writer (the bun:sqlite
    // writer lock serializes the bodies of transactions, so only one of two
    // racing flushes can be inside the critical section at a time).
    this.stateDb.transaction(() => {
      const latestInTx = this.getLatestSnapshot(input.streamId);
      if (latestInTx && latestInTx.version_hash === versionHash) {
        resultId = latestInTx.id;
        created = false;
        return;
      }
      this.stateDb.run(
        `INSERT INTO file_snapshot (id, stream_id, worktree_path, version_hash, source, created_at)
         VALUES (?, ?, ?, ?, ?, ?)`,
        newId,
        input.streamId,
        input.worktreePath,
        versionHash,
        input.source,
        now,
      );
      for (const [path, entry] of entries) {
        this.stateDb.run(
          `INSERT INTO snapshot_entry (snapshot_id, path, hash, mtime_ms, size, state)
           VALUES (?, ?, ?, ?, ?, ?)`,
          newId,
          path,
          entry.hash,
          entry.mtime_ms,
          entry.size,
          entry.state,
        );
      }
    });
    return { id: resultId, created, versionHash };
  }

  private buildEntries(
    input: FlushInput,
    baseline: Record<string, SnapshotEntry>,
    ignore: (relpath: string) => boolean,
  ): Array<[string, SnapshotEntry]> {
    if (input.dirtyPaths && input.dirtyPaths.length > 0) {
      // Optimizer hint path: only rescan the dirty set, carry everything else
      // forward from the previous snapshot.
      const merged: Record<string, SnapshotEntry> = { ...baseline };
      const unique = Array.from(new Set(input.dirtyPaths)).sort();
      for (const rel of unique) {
        const entry = this.captureEntry(input.worktreePath, rel);
        const prev = baseline[rel];
        if (entry) {
          merged[rel] = entry;
        } else if (prev && prev.state !== "deleted") {
          // Tracked file disappeared: emit tombstone against the previous
          // snapshot so the diff reads as a deletion. A stale tombstone (file
          // was already gone) is dropped so snapshots don't carry ancient
          // ghosts forever.
          merged[rel] = { hash: "", mtime_ms: 0, size: 0, state: "deleted" };
        } else {
          delete merged[rel];
        }
      }
      return toSortedEntryList(merged);
    }
    if (input.dirtyPaths && input.dirtyPaths.length === 0) {
      // Explicit "nothing changed" hint — carry baseline forward; dedup will
      // catch an unchanged version_hash.
      return toSortedEntryList(baseline);
    }
    // Full walk.
    const captured: Record<string, SnapshotEntry> = {};
    walkAll(input.worktreePath, "", ignore, (rel, st) => {
      const entry = this.captureEntryFromStat(input.worktreePath, rel, st);
      if (entry) captured[rel] = entry;
    });
    return toSortedEntryList(captured);
  }

  /**
   * Capture a single entry from disk. Returns null if the file doesn't exist
   * on disk (or isn't a regular file) — callers decide whether to emit a
   * tombstone or skip the path based on the baseline.
   */
  private captureEntry(worktreePath: string, relpath: string): SnapshotEntry | null {
    const abs = resolve(worktreePath, relpath);
    if (!existsSync(abs)) return null;
    let st;
    try {
      st = statSync(abs);
    } catch {
      return null;
    }
    if (!st.isFile()) return null;
    return this.captureEntryFromStat(worktreePath, relpath, st);
  }

  private captureEntryFromStat(
    worktreePath: string,
    relpath: string,
    st: import("node:fs").Stats,
  ): SnapshotEntry | null {
    const abs = resolve(worktreePath, relpath);
    if (!st.isFile()) return null;
    if (st.size > this.maxFileBytes) {
      return { hash: "", mtime_ms: Math.floor(st.mtimeMs), size: st.size, state: "oversize" };
    }
    const content = readFileSync(abs);
    const hash = this.writeBlob(content);
    return { hash, mtime_ms: Math.floor(st.mtimeMs), size: st.size, state: "present" };
  }

  getSnapshot(id: string): FileSnapshot | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      "SELECT * FROM file_snapshot WHERE id = ? LIMIT 1",
      id,
    );
    return row ? rowToSnapshot(row) : null;
  }

  getLatestSnapshot(streamId: string): FileSnapshot | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT * FROM file_snapshot WHERE stream_id = ?
       ORDER BY created_at DESC, rowid DESC LIMIT 1`,
      streamId,
    );
    return row ? rowToSnapshot(row) : null;
  }

  /** The snapshot immediately preceding `id` for its stream (by time), or null. */
  getPreviousSnapshot(id: string): FileSnapshot | null {
    const snap = this.getSnapshot(id);
    if (!snap) return null;
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT * FROM file_snapshot
       WHERE stream_id = ? AND (created_at < ? OR (created_at = ? AND rowid < (SELECT rowid FROM file_snapshot WHERE id = ?)))
       ORDER BY created_at DESC, rowid DESC LIMIT 1`,
      snap.stream_id,
      snap.created_at,
      snap.created_at,
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

  /** Resolved hash for `relpath` at snapshot `id`, or null if deleted/absent. */
  resolvePath(id: string, relpath: string): string | null {
    const entry = this.loadManifestEntries(id)[relpath];
    if (!entry || entry.state !== "present" || !entry.hash) return null;
    return entry.hash;
  }

  /** Resolved entry (with state) for `relpath` at `id`, or null if absent. */
  resolveEntry(id: string, relpath: string): SnapshotEntry | null {
    return this.loadManifestEntries(id)[relpath] ?? null;
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
   * Summary for the UI: row + per-file entries + A/M/D classification
   * relative to `previousSnapshotId` (defaults to the preceding snapshot in
   * time for this stream).
   */
  getSnapshotSummary(snapshotId: string, previousSnapshotId?: string | null): SnapshotSummary | null {
    const snap = this.getSnapshot(snapshotId);
    if (!snap) return null;
    const entries = this.loadManifestEntries(snapshotId);
    const previousId = previousSnapshotId === undefined
      ? this.getPreviousSnapshot(snapshotId)?.id ?? null
      : previousSnapshotId;
    const previousEntries = previousId ? this.loadManifestEntries(previousId) : {};
    const files: Record<string, SnapshotFileRow> = {};
    let created = 0;
    let updated = 0;
    let deleted = 0;
    const allPaths = new Set<string>([
      ...Object.keys(entries),
      ...Object.keys(previousEntries),
    ]);
    for (const path of allPaths) {
      const entry = entries[path];
      const prev = previousEntries[path];
      if (!entry || entry.state === "deleted") {
        if (prev && prev.state !== "deleted") {
          deleted++;
          files[path] = {
            entry: entry ?? { hash: "", mtime_ms: 0, size: 0, state: "deleted" },
            kind: "deleted",
          };
        }
        continue;
      }
      if (!prev || prev.state === "deleted") {
        if (entry.state !== "oversize") created++;
        files[path] = { entry, kind: "created" };
        continue;
      }
      const hashChanged = prev.state === "present" && entry.state === "present" && prev.hash !== entry.hash;
      const statChanged = prev.mtime_ms !== entry.mtime_ms || prev.size !== entry.size;
      const stateChanged = prev.state !== entry.state;
      if (hashChanged || statChanged || stateChanged) {
        updated++;
        files[path] = { entry, kind: "updated" };
      }
    }
    return {
      snapshot: snap,
      previousSnapshotId: previousId,
      files,
      counts: { created, updated, deleted },
    };
  }

  getSnapshotPairDiff(
    beforeSnapshotId: string | null,
    afterSnapshotId: string,
    path: string,
  ): SnapshotDiffResult {
    return this.diffPath(beforeSnapshotId, afterSnapshotId, path);
  }

  /**
   * Walk the worktree and compare (mtime_ms, size) against the entry map for
   * `snapshotId`. Returns paths whose stat differs, or that exist on disk
   * but not in the manifest, or vice versa.
   */
  reconcileWorktree(
    snapshotId: string,
    worktreePath: string,
    ignore: (relpath: string) => boolean,
  ): string[] {
    const entries = this.loadManifestEntries(snapshotId);
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
   * Snapshots visible in Local History: excludes the very first snapshot per
   * stream (the initial baseline has nothing to diff against and would only
   * add noise). The baseline still exists in the DB and acts as the
   * "previous" for the second snapshot's summary.
   *
   * Each row is enriched with a `label` + `label_kind` derived from joined
   * work-item efforts / agent turns, so the UI can render a meaningful name
   * ("<task title> end" / "<turn prompt>") without N+1 follow-up queries.
   * When no effort or turn references the snapshot, label is null and the
   * UI falls back to the `source` column. Only the end-of-effort side is
   * labeled — a snapshot that is merely the start point of an effort adds
   * no information to the history view (effort starts are already implicit
   * in the previous row).
   */
  listSnapshotsForStream(streamId: string, limit = 100): FileSnapshot[] {
    // Correlated subqueries (rather than LEFT JOIN + GROUP BY) so that when
    // multiple efforts or turns reference the same snapshot, the picked title
    // / prompt is deterministic: most recent effort or turn wins. Ties
    // broken by rowid for reproducibility.
    const rows = this.stateDb.all<Record<string, unknown>>(
      `SELECT f.*,
              (
                SELECT wi.title FROM work_item_effort e
                JOIN work_items wi ON wi.id = e.work_item_id
                WHERE e.end_snapshot_id = f.id
                ORDER BY COALESCE(e.ended_at, '') DESC, e.rowid DESC LIMIT 1
              ) AS effort_end_title,
              (
                SELECT t.prompt FROM agent_turn t
                WHERE t.end_snapshot_id = f.id
                ORDER BY COALESCE(t.ended_at, '') DESC, t.rowid DESC LIMIT 1
              ) AS turn_end_prompt,
              (
                SELECT t.prompt FROM agent_turn t
                WHERE t.start_snapshot_id = f.id
                ORDER BY t.started_at DESC, t.rowid DESC LIMIT 1
              ) AS turn_start_prompt
       FROM file_snapshot f
       WHERE f.stream_id = ?
         AND EXISTS (
           SELECT 1 FROM file_snapshot earlier
           WHERE earlier.stream_id = f.stream_id
             AND (earlier.created_at < f.created_at
                  OR (earlier.created_at = f.created_at AND earlier.rowid < f.rowid))
         )
       ORDER BY f.created_at DESC, f.rowid DESC
       LIMIT ?`,
      streamId,
      limit,
    );
    return rows.map((row) => {
      const snap = rowToSnapshot(row);
      const { label, kind } = deriveSnapshotLabel(row);
      snap.label = label;
      snap.label_kind = kind;
      return snap;
    });
  }

  /**
   * Drop snapshots older than `retentionDays`. Always keeps the most recent
   * snapshot per stream. `retentionDays === 0` disables pruning.
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

    const blobsDeleted = this.gcBlobs();
    return { snapshotsDeleted: toDelete.length, blobsDeleted };
  }

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
}

function toSortedEntryList(map: Record<string, SnapshotEntry>): Array<[string, SnapshotEntry]> {
  return Object.keys(map)
    .sort()
    .map((path) => [path, map[path]!] as [string, SnapshotEntry]);
}

/**
 * SHA-256 over a canonical encoding of the entry set. The encoding is a
 * newline-separated list of `path\thash\tsize\tstate` tuples sorted by path.
 * `mtime_ms` is deliberately excluded so touching a file without changing its
 * content doesn't produce a new snapshot.
 */
export function computeVersionHash(entries: Array<[string, SnapshotEntry]>): string {
  const h = createHash("sha256");
  for (const [path, entry] of entries) {
    h.update(path);
    h.update("\t");
    h.update(entry.hash);
    h.update("\t");
    h.update(String(entry.size));
    h.update("\t");
    h.update(entry.state);
    h.update("\n");
  }
  return h.digest("hex");
}

function deriveSnapshotLabel(row: Record<string, unknown>): {
  label: string | null;
  kind: "task" | "turn" | "system" | null;
} {
  const effortEnd = asString(row.effort_end_title);
  if (effortEnd) return { label: `${effortEnd} — end`, kind: "task" };
  const turnEnd = asString(row.turn_end_prompt);
  if (turnEnd) return { label: firstLine(turnEnd), kind: "turn" };
  const turnStart = asString(row.turn_start_prompt);
  if (turnStart) return { label: firstLine(turnStart), kind: "turn" };
  return { label: null, kind: null };
}

function asString(value: unknown): string | null {
  if (value == null) return null;
  const s = String(value).trim();
  return s.length > 0 ? s : null;
}

function firstLine(text: string): string {
  const newline = text.indexOf("\n");
  return newline === -1 ? text : text.slice(0, newline);
}

function rowToSnapshot(row: Record<string, unknown>): FileSnapshot {
  return {
    id: String(row.id ?? ""),
    stream_id: String(row.stream_id ?? ""),
    worktree_path: String(row.worktree_path ?? ""),
    version_hash: String(row.version_hash ?? ""),
    source: String(row.source ?? "startup") as SnapshotSource,
    created_at: String(row.created_at ?? ""),
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
