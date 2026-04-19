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
import { join, relative, resolve } from "node:path";
import { createId } from "../core/ids.js";
import type { Logger } from "../core/logger.js";
import { getStateDatabase } from "./state-db.js";

export type SnapshotKind = "turn-start" | "turn-end";

export interface ManifestEntry {
  hash: string;
  mtime_ms: number;
  size: number;
  deleted?: boolean;
}

export interface SnapshotFileRow {
  entry: ManifestEntry;
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
  manifest_path: string;
  created_at: string;
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

interface ManifestFile {
  snapshot_id: string;
  created_at: string;
  kind: SnapshotKind;
  worktree_path: string;
  stream_id: string;
  turn_id: string | null;
  batch_id: string | null;
  parent_snapshot_id: string | null;
  entries: Record<string, ManifestEntry>;
}

// Files larger than this get a sentinel hash in the manifest instead of being
// read into memory — accidental big-binary edits shouldn't balloon the store.
const MAX_BLOB_SIZE = 5 * 1024 * 1024;
const OVERSIZE_SENTINEL = "oversize";

export class SnapshotStore {
  private readonly stateDb;
  private readonly rootDir: string;
  private readonly objectsDir: string;
  private readonly manifestsDir: string;

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.rootDir = join(projectDir, ".newde", "snapshots");
    this.objectsDir = join(this.rootDir, "objects");
    this.manifestsDir = join(this.rootDir, "manifests");
    mkdirSync(this.objectsDir, { recursive: true });
    mkdirSync(this.manifestsDir, { recursive: true });
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
    const manifestRelPath = join("manifests", `${id}.json`);
    const entries: Record<string, ManifestEntry> = {};

    const unique = Array.from(new Set(input.dirtyPaths)).sort();
    for (const relpath of unique) {
      const abs = resolve(input.worktreePath, relpath);
      if (!existsSync(abs)) {
        entries[relpath] = {
          hash: "",
          mtime_ms: 0,
          size: 0,
          deleted: true,
        };
        continue;
      }
      let st;
      try {
        st = statSync(abs);
      } catch {
        continue;
      }
      if (!st.isFile()) continue;
      if (st.size > MAX_BLOB_SIZE) {
        entries[relpath] = {
          hash: OVERSIZE_SENTINEL,
          mtime_ms: Math.floor(st.mtimeMs),
          size: st.size,
        };
        continue;
      }
      const content = readFileSync(abs);
      const hash = this.writeBlob(content);
      entries[relpath] = {
        hash,
        mtime_ms: Math.floor(st.mtimeMs),
        size: st.size,
      };
    }

    if (Object.keys(entries).length === 0) return null;

    const manifest: ManifestFile = {
      snapshot_id: id,
      created_at: now,
      kind: input.kind,
      worktree_path: input.worktreePath,
      stream_id: input.streamId,
      turn_id: input.turnId,
      batch_id: input.batchId,
      parent_snapshot_id: input.parentSnapshotId,
      entries,
    };
    writeFileSync(join(this.rootDir, manifestRelPath), JSON.stringify(manifest, null, 2));

    this.stateDb.run(
      `INSERT INTO file_snapshot (
        id, stream_id, worktree_path, kind, turn_id, batch_id,
        parent_snapshot_id, manifest_path, created_at
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      id,
      input.streamId,
      input.worktreePath,
      input.kind,
      input.turnId,
      input.batchId,
      input.parentSnapshotId,
      manifestRelPath,
      now,
    );
    return id;
  }

  getSnapshot(id: string): FileSnapshot | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      "SELECT * FROM file_snapshot WHERE id = ? LIMIT 1",
      id,
    );
    return row ? rowToSnapshot(row) : null;
  }

  loadManifestEntries(id: string): Record<string, ManifestEntry> {
    const snap = this.getSnapshot(id);
    if (!snap) return {};
    const file = join(this.rootDir, snap.manifest_path);
    if (!existsSync(file)) return {};
    const manifest = JSON.parse(readFileSync(file, "utf8")) as ManifestFile;
    return manifest.entries ?? {};
  }

  /** Merge entries walking the parent chain (newest wins). Tombstones kept. */
  resolveEntries(id: string): Record<string, ManifestEntry> {
    const merged: Record<string, ManifestEntry> = {};
    const chain: string[] = [];
    let cursor: string | null = id;
    while (cursor) {
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
    return merged;
  }

  /** Resolved hash for `relpath` at snapshot `id`, or null if deleted/absent. */
  resolvePath(id: string, relpath: string): string | null {
    let cursor: string | null = id;
    while (cursor) {
      const entries = this.loadManifestEntries(cursor);
      const entry = entries[relpath];
      if (entry) {
        if (entry.deleted || !entry.hash) return null;
        return entry.hash;
      }
      const snap = this.getSnapshot(cursor);
      cursor = snap?.parent_snapshot_id ?? null;
    }
    return null;
  }

  diffPath(beforeId: string | null, afterId: string, relpath: string): { before: string | null; after: string | null } {
    const beforeHash = beforeId ? this.resolvePath(beforeId, relpath) : null;
    const afterHash = this.resolvePath(afterId, relpath);
    return {
      before: beforeHash ? this.readBlobAsText(beforeHash) : null,
      after: afterHash ? this.readBlobAsText(afterHash) : null,
    };
  }

  private readBlobAsText(hash: string): string | null {
    if (hash === OVERSIZE_SENTINEL) return null;
    try {
      return this.readBlob(hash).toString("utf8");
    } catch {
      return null;
    }
  }

  /**
   * Walk the worktree and compare (mtime_ms, size) against the resolved
   * entry map for `snapshotId`. Any path whose stat differs — or that
   * exists in the manifest but not on disk — is returned.
   */
  reconcileWorktree(
    snapshotId: string,
    worktreePath: string,
    ignore: (relpath: string) => boolean,
  ): string[] {
    const entries = this.resolveEntries(snapshotId);
    const dirty = new Set<string>();
    const seen = new Set<string>();

    walk(worktreePath, "", ignore, (rel, st) => {
      seen.add(rel);
      const entry = entries[rel];
      if (!entry || entry.deleted) {
        dirty.add(rel);
        return;
      }
      if (entry.size !== st.size || entry.mtime_ms !== Math.floor(st.mtimeMs)) {
        dirty.add(rel);
      }
    });

    for (const [rel, entry] of Object.entries(entries)) {
      if (entry.deleted) continue;
      if (!seen.has(rel)) dirty.add(rel);
    }
    return Array.from(dirty).sort();
  }

  /**
   * Summarize a snapshot for the UI: the row, the manifest, and A/M/D
   * classification of each entry against the parent chain. `created` =
   * entry present here but not in any ancestor. `deleted` = tombstone
   * entry. `updated` = content differs from the ancestor resolution.
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
      if (entry.deleted) {
        kind = "deleted";
        deleted++;
      } else if (!parentEntries[path] || parentEntries[path].deleted) {
        kind = "created";
        created++;
      } else if (parentEntries[path].hash !== entry.hash) {
        kind = "updated";
        updated++;
      } else {
        // Stat-only change (mtime bumped but content identical). Record as
        // updated so the UI still surfaces it, but don't bump the update
        // count so the header stays accurate.
        kind = "updated";
      }
      files[path] = { entry, kind };
    }
    return {
      snapshot: snap,
      files,
      counts: { created, updated, deleted },
    };
  }

  /**
   * Resolved before/after contents for `path` between this snapshot's
   * parent chain and this snapshot itself.
   */
  getSnapshotFileDiff(snapshotId: string, path: string): { before: string | null; after: string | null } {
    const snap = this.getSnapshot(snapshotId);
    if (!snap) return { before: null, after: null };
    const beforeHash = snap.parent_snapshot_id
      ? this.resolvePath(snap.parent_snapshot_id, path)
      : null;
    const afterHash = this.resolvePath(snapshotId, path);
    return {
      before: beforeHash ? this.readBlobAsText(beforeHash) : null,
      after: afterHash ? this.readBlobAsText(afterHash) : null,
    };
  }

  /** Arbitrary-pair variant: resolve `path` in each side independently. */
  getSnapshotPairDiff(
    beforeSnapshotId: string | null,
    afterSnapshotId: string,
    path: string,
  ): { before: string | null; after: string | null } {
    return this.diffPath(beforeSnapshotId, afterSnapshotId, path);
  }

  /**
   * Drop snapshots older than `retentionDays` while preserving each
   * stream's most recent snapshot (and anything `streams.current_snapshot_id`
   * points at) no matter its age. After deleting snapshot rows + manifest
   * files, sweep `objects/` for blobs that are no longer referenced by any
   * surviving manifest. `retentionDays === 0` disables pruning entirely.
   *
   * Descendant snapshots whose parent was just deleted keep pointing at a
   * missing id — `resolvePath` simply stops walking when the parent row is
   * gone, so the oldest surviving snapshot effectively becomes a new
   * baseline for the files it touches. This is the trade-off: ancient
   * history drops, recent diffs stay intact.
   */
  cleanupOldSnapshots(retentionDays: number): { snapshotsDeleted: number; blobsDeleted: number } {
    if (retentionDays <= 0) return { snapshotsDeleted: 0, blobsDeleted: 0 };
    const cutoff = new Date(Date.now() - retentionDays * 86400_000).toISOString();
    const keep = new Set<string>();

    // Pin the most recent snapshot per stream.
    const latestRows = this.stateDb.all<{ id: string }>(
      `SELECT id FROM file_snapshot f1
       WHERE created_at = (
         SELECT MAX(created_at) FROM file_snapshot f2 WHERE f2.stream_id = f1.stream_id
       )`,
    );
    for (const row of latestRows) keep.add(row.id);

    // Pin whatever each stream currently points at (defensive — should
    // overlap with the "latest" set but handle the edge case of a manually
    // set pointer that isn't the row with MAX(created_at)).
    const pointedRows = this.stateDb.all<{ current_snapshot_id: string | null }>(
      `SELECT current_snapshot_id FROM streams WHERE current_snapshot_id IS NOT NULL`,
    );
    for (const row of pointedRows) {
      if (row.current_snapshot_id) keep.add(row.current_snapshot_id);
    }

    const staleRows = this.stateDb.all<{ id: string; manifest_path: string }>(
      `SELECT id, manifest_path FROM file_snapshot WHERE created_at < ?`,
      cutoff,
    );
    const toDelete = staleRows.filter((row) => !keep.has(row.id));
    if (toDelete.length === 0) return { snapshotsDeleted: 0, blobsDeleted: 0 };

    this.stateDb.transaction(() => {
      for (const row of toDelete) {
        this.stateDb.run("DELETE FROM file_snapshot WHERE id = ?", row.id);
      }
    });
    // Remove the manifest JSON files outside the transaction (the DB row is
    // the source of truth; orphaned files on disk are harmless noise, but
    // we clean them up for tidiness).
    for (const row of toDelete) {
      const manifestAbs = join(this.rootDir, row.manifest_path);
      try {
        if (existsSync(manifestAbs)) unlinkSyncSafe(manifestAbs);
      } catch {
        // best effort
      }
    }

    const blobsDeleted = this.gcBlobs();
    return { snapshotsDeleted: toDelete.length, blobsDeleted };
  }

  /**
   * Sweep `objects/` for blobs whose sha is not referenced by any
   * surviving manifest. Returns the number of files removed. Invoked after
   * snapshot deletion; safe to call on its own.
   */
  gcBlobs(): number {
    const referenced = new Set<string>();
    const rows = this.stateDb.all<{ manifest_path: string }>(
      "SELECT manifest_path FROM file_snapshot",
    );
    for (const row of rows) {
      const file = join(this.rootDir, row.manifest_path);
      if (!existsSync(file)) continue;
      try {
        const manifest = JSON.parse(readFileSync(file, "utf8")) as ManifestFile;
        for (const entry of Object.values(manifest.entries ?? {})) {
          if (entry.hash && entry.hash !== OVERSIZE_SENTINEL && !entry.deleted) {
            referenced.add(entry.hash);
          }
        }
      } catch {
        // unreadable manifest: be conservative — bail on gc so we don't
        // accidentally delete blobs we can't account for.
        return 0;
      }
    }

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
          unlinkSyncSafe(join(shardPath, entry.name));
          removed++;
        } catch {
          // best effort
        }
      }
    }
    return removed;
  }

  listSnapshotsForStream(streamId: string, limit = 100): FileSnapshot[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        "SELECT * FROM file_snapshot WHERE stream_id = ? ORDER BY created_at DESC, rowid DESC LIMIT ?",
        streamId,
        limit,
      )
      .map(rowToSnapshot);
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
    manifest_path: String(row.manifest_path ?? ""),
    created_at: String(row.created_at ?? ""),
  };
}

function unlinkSyncSafe(path: string): void {
  try {
    unlinkSync(path);
  } catch {
    // ignore
  }
}

function walk(
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
      walk(root, childRel, ignore, onFile);
      continue;
    }
    if (!entry.isFile()) continue;
    let st;
    try {
      st = statSync(join(root, childRel));
    } catch {
      continue;
    }
    if (st.size > MAX_BLOB_SIZE) continue;
    onFile(childRel, st);
  }
}
