import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { createId } from "../core/ids.js";
import type { Logger } from "../core/logger.js";
import { getStateDatabase } from "./state-db.js";

export type PaneKind = "working" | "talking";
export type StreamBranchSource = "local" | "remote" | "new";

export interface Stream {
  id: string;
  title: string;
  summary: string;
  branch: string;
  branch_ref: string;
  branch_source: StreamBranchSource;
  worktree_path: string;
  created_at: string;
  updated_at: string;
  panes: {
    working: string;
    talking: string;
  };
  resume: {
    working_session_id: string;
    talking_session_id: string;
  };
}

export class StreamStore {
  private readonly rootDir: string;
  private readonly legacyDir: string;
  private readonly legacyStatePath: string;
  private readonly stateDb;

  constructor(
    private readonly projectDir: string,
    private readonly logger?: Logger,
  ) {
    this.rootDir = join(projectDir, ".newde");
    this.legacyDir = join(this.rootDir, "streams");
    this.legacyStatePath = join(this.rootDir, "state.json");
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.migrateLegacyIfNeeded();
  }

  list(): Stream[] {
    return this.stateDb
      .all<Record<string, unknown>>("SELECT * FROM streams ORDER BY created_at, rowid")
      .map(rowToStream);
  }

  get(id: string): Stream | undefined {
    const row = this.stateDb.get<Record<string, unknown>>("SELECT * FROM streams WHERE id = ? LIMIT 1", id);
    return row ? rowToStream(row) : undefined;
  }

  getCurrentStreamId(): string | null {
    const row = this.stateDb.get<{ current_stream_id: string | null }>(
      "SELECT current_stream_id FROM runtime_state WHERE id = 1 LIMIT 1",
    );
    return row?.current_stream_id ?? null;
  }

  getCurrent(): Stream | undefined {
    const id = this.getCurrentStreamId();
    return id ? this.get(id) : undefined;
  }

  setCurrentStreamId(id: string) {
    if (!this.get(id)) throw new Error(`unknown stream: ${id}`);
    this.stateDb.run("UPDATE runtime_state SET current_stream_id = ? WHERE id = 1", id);
    this.logger?.info("updated current stream", { streamId: id });
  }

  ensureCurrentStreamId(fallbackId: string) {
    const existing = this.getCurrentStreamId();
    if (existing && this.get(existing)) return existing;
    this.setCurrentStreamId(fallbackId);
    return fallbackId;
  }

  findByBranch(branch: string): Stream | undefined {
    const row = this.stateDb.get<Record<string, unknown>>(
      "SELECT * FROM streams WHERE branch = ? ORDER BY created_at LIMIT 1",
      branch,
    );
    return row ? rowToStream(row) : undefined;
  }

  findByPane(target: string): { stream: Stream; pane: PaneKind } | undefined {
    const row = this.stateDb.get<Record<string, unknown>>(
      "SELECT * FROM streams WHERE working_pane = ? OR talking_pane = ? LIMIT 1",
      target,
      target,
    );
    if (!row) return undefined;
    const stream = rowToStream(row);
    return { stream, pane: stream.panes.working === target ? "working" : "talking" };
  }

  create(input: {
    title: string;
    summary?: string;
    branch: string;
    branchRef?: string;
    branchSource?: StreamBranchSource;
    worktreePath: string;
    projectBase: string;
  }): Stream {
    const id = createId("s");
    const now = new Date().toISOString();
    const stream: Stream = {
      id,
      title: input.title,
      summary: input.summary ?? "",
      branch: input.branch,
      branch_ref: input.branchRef ?? `refs/heads/${input.branch}`,
      branch_source: input.branchSource ?? "local",
      worktree_path: input.worktreePath,
      created_at: now,
      updated_at: now,
      panes: {
        working: `newde-${input.projectBase}:working-${id}`,
        talking: `newde-${input.projectBase}:talking-${id}`,
      },
      resume: {
        working_session_id: "",
        talking_session_id: "",
      },
    };
    this.save(stream);
    return stream;
  }

  save(stream: Stream) {
    this.stateDb.run(
      `INSERT INTO streams (
        id, title, summary, branch, branch_ref, branch_source, worktree_path,
        working_pane, talking_pane, working_session_id, talking_session_id,
        created_at, updated_at
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(id) DO UPDATE SET
        title = excluded.title,
        summary = excluded.summary,
        branch = excluded.branch,
        branch_ref = excluded.branch_ref,
        branch_source = excluded.branch_source,
        worktree_path = excluded.worktree_path,
        working_pane = excluded.working_pane,
        talking_pane = excluded.talking_pane,
        working_session_id = excluded.working_session_id,
        talking_session_id = excluded.talking_session_id,
        created_at = excluded.created_at,
        updated_at = excluded.updated_at`,
      stream.id,
      stream.title,
      stream.summary,
      stream.branch,
      stream.branch_ref,
      stream.branch_source,
      stream.worktree_path,
      stream.panes.working,
      stream.panes.talking,
      stream.resume.working_session_id,
      stream.resume.talking_session_id,
      stream.created_at,
      stream.updated_at,
    );
  }

  getCurrentSnapshotId(streamId: string): string | null {
    const row = this.stateDb.get<{ current_snapshot_id: string | null }>(
      "SELECT current_snapshot_id FROM streams WHERE id = ? LIMIT 1",
      streamId,
    );
    return row?.current_snapshot_id ?? null;
  }

  setCurrentSnapshotId(streamId: string, snapshotId: string): void {
    this.stateDb.run(
      "UPDATE streams SET current_snapshot_id = ? WHERE id = ?",
      snapshotId,
      streamId,
    );
  }

  update(streamId: string, mutate: (stream: Stream) => Stream): Stream {
    const existing = this.get(streamId);
    if (!existing) throw new Error(`unknown stream: ${streamId}`);
    const updated = mutate(existing);
    updated.updated_at = new Date().toISOString();
    this.save(updated);
    return updated;
  }

  private migrateLegacyIfNeeded(): void {
    const row = this.stateDb.get<{ c: number }>("SELECT COUNT(*) AS c FROM streams");
    if ((row?.c ?? 0) > 0) return;
    if (!existsSync(this.legacyDir)) return;

    const imported: Stream[] = [];
    for (const name of readdirSync(this.legacyDir)) {
      if (!name.endsWith(".yml")) continue;
      try {
        const parsed = withDefaults(parseYaml(readFileSync(join(this.legacyDir, name), "utf8")), this.projectDir);
        if (!parsed.id) continue;
        imported.push(parsed);
      } catch (error) {
        this.logger?.warn("failed to import legacy stream file", {
          file: name,
          error: errorMessage(error),
        });
      }
    }
    if (imported.length === 0) return;
    this.stateDb.transaction(() => {
      for (const stream of imported) this.save(stream);
      const currentId = this.readLegacyCurrentStreamId();
      const fallback = currentId && imported.some((stream) => stream.id === currentId) ? currentId : imported[0]!.id;
      this.stateDb.run("UPDATE runtime_state SET current_stream_id = ? WHERE id = 1", fallback);
    });
    this.logger?.info("imported legacy streams into sqlite", { count: imported.length });
  }

  private readLegacyCurrentStreamId(): string | null {
    if (!existsSync(this.legacyStatePath)) return null;
    try {
      const parsed = JSON.parse(readFileSync(this.legacyStatePath, "utf8"));
      return typeof parsed.currentStreamId === "string" ? parsed.currentStreamId : null;
    } catch {
      return null;
    }
  }
}

function rowToStream(row: Record<string, unknown>): Stream {
  return {
    id: String(row.id ?? ""),
    title: String(row.title ?? ""),
    summary: String(row.summary ?? ""),
    branch: String(row.branch ?? ""),
    branch_ref: String(row.branch_ref ?? ""),
    branch_source: String(row.branch_source ?? "local") as StreamBranchSource,
    worktree_path: String(row.worktree_path ?? ""),
    created_at: String(row.created_at ?? new Date(0).toISOString()),
    updated_at: String(row.updated_at ?? row.created_at ?? new Date(0).toISOString()),
    panes: {
      working: String(row.working_pane ?? ""),
      talking: String(row.talking_pane ?? ""),
    },
    resume: {
      working_session_id: String(row.working_session_id ?? ""),
      talking_session_id: String(row.talking_session_id ?? ""),
    },
  };
}

function parseYaml(text: string): Stream {
  const out: Record<string, any> = { panes: {}, resume: {} };
  const lines = text.split("\n");
  let section: "panes" | "resume" | null = null;
  for (const line of lines) {
    if (!line.trim() || line.trim().startsWith("#")) continue;
    const sectionMatch = /^([a-z_]+):\s*$/.exec(line);
    if (sectionMatch) {
      const key = sectionMatch[1];
      section = key === "panes" || key === "resume" ? key : null;
      if (section) out[section] = out[section] ?? {};
      continue;
    }
    const match = /^(\s*)([a-z_]+):\s*(.*)$/.exec(line);
    if (!match) continue;
    const [, indent, key, rawVal] = match;
    const value = rawVal.trim();
    if (!value) continue;
    const parsed = value.startsWith('"') ? JSON.parse(value) : value;
    if (indent.length > 0) {
      if (section) out[section][key] = parsed;
    } else {
      out[key] = parsed;
    }
  }
  return out as unknown as Stream;
}

function withDefaults(stream: Partial<Stream>, projectDir: string): Stream {
  return {
    id: stream.id ?? "",
    title: stream.title ?? stream.branch ?? "",
    summary: stream.summary ?? "",
    branch: stream.branch ?? "",
    branch_ref: stream.branch_ref ?? (stream.branch ? `refs/heads/${stream.branch}` : ""),
    branch_source: stream.branch_source ?? "local",
    worktree_path: stream.worktree_path ?? projectDir,
    created_at: stream.created_at ?? new Date(0).toISOString(),
    updated_at: stream.updated_at ?? stream.created_at ?? new Date(0).toISOString(),
    panes: {
      working: stream.panes?.working ?? "",
      talking: stream.panes?.talking ?? "",
    },
    resume: {
      working_session_id: stream.resume?.working_session_id ?? "",
      talking_session_id: stream.resume?.talking_session_id ?? "",
    },
  };
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
