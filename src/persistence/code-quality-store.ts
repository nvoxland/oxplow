import type { Logger } from "../core/logger.js";
import { getStateDatabase } from "./state-db.js";
import { StoreEmitter } from "./store-emitter.js";

export type CodeQualityTool = "lizard" | "jscpd";
export type CodeQualityScope = "codebase" | "diff";
export type CodeQualityScanStatus = "running" | "completed" | "failed";

export type CodeQualityFindingKind =
  | "complexity"
  | "function-length"
  | "parameter-count"
  | "duplicate-block";

export interface CodeQualityFinding {
  path: string;
  startLine: number;
  endLine: number;
  kind: CodeQualityFindingKind;
  metricValue: number;
  extra: Record<string, unknown> | null;
}

export interface CodeQualityFindingRow extends CodeQualityFinding {
  id: number;
  scanId: number;
}

export interface CodeQualityScanRow {
  id: number;
  stream_id: string;
  tool: CodeQualityTool;
  scope: CodeQualityScope;
  base_ref: string | null;
  status: CodeQualityScanStatus;
  error_message: string | null;
  started_at: string;
  completed_at: string | null;
}

export interface StartScanInput {
  streamId: string;
  tool: CodeQualityTool;
  scope: CodeQualityScope;
  baseRef?: string | null;
  startedAt?: string;
}

export interface ListLatestFindingsQuery {
  streamId: string;
  tool?: CodeQualityTool;
  paths?: string[];
}

export interface CodeQualityChange {
  kind: "started" | "completed" | "failed";
  scanId: number;
  streamId: string;
  tool: CodeQualityTool;
  scope: CodeQualityScope;
}

export interface CodeQualityStoreOptions {
  /** Keep this many most-recent scans per (stream, tool, scope). Default 10. */
  keepLast?: number;
}

interface RawFindingRow {
  id: number;
  scan_id: number;
  path: string;
  start_line: number;
  end_line: number;
  kind: string;
  metric_value: number;
  extra_json: string | null;
}

/**
 * Persists code-quality scan runs and their findings. Scans are scoped
 * per (stream, tool, scope); each `completeScan` retires older scans
 * for that triple beyond `keepLast` (default 10) so the table stays
 * small without external pruning. Findings of pruned scans are deleted
 * in the same transaction.
 *
 * The store does NOT run external CLIs; the runtime owns subprocess
 * orchestration and hands normalized findings here.
 */
export class CodeQualityStore {
  private readonly stateDb;
  private readonly emitter: StoreEmitter<CodeQualityChange>;
  private readonly keepLast: number;

  constructor(projectDir: string, logger?: Logger, options: CodeQualityStoreOptions = {}) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.emitter = new StoreEmitter("code-quality", logger);
    this.keepLast = options.keepLast ?? 10;
  }

  subscribe(listener: (change: CodeQualityChange) => void): () => void {
    return this.emitter.subscribe(listener);
  }

  startScan(input: StartScanInput): number {
    const startedAt = input.startedAt ?? new Date().toISOString();
    const row = this.stateDb.get<{ id: number }>(
      `INSERT INTO code_quality_scan (stream_id, tool, scope, base_ref, status, started_at)
       VALUES (?, ?, ?, ?, 'running', ?)
       RETURNING id`,
      input.streamId,
      input.tool,
      input.scope,
      input.baseRef ?? null,
      startedAt,
    );
    if (!row) throw new Error("INSERT into code_quality_scan returned no row");
    const scanId = row.id;
    this.emitter.emit({
      kind: "started",
      scanId,
      streamId: input.streamId,
      tool: input.tool,
      scope: input.scope,
    });
    return scanId;
  }

  completeScan(scanId: number, findings: CodeQualityFinding[]): void {
    const scan = this.requireScan(scanId);
    const completedAt = new Date().toISOString();
    this.stateDb.transaction(() => {
      this.stateDb.run(
        `UPDATE code_quality_scan SET status = 'completed', completed_at = ? WHERE id = ?`,
        completedAt,
        scanId,
      );
      for (const f of findings) {
        this.stateDb.run(
          `INSERT INTO code_quality_finding
            (scan_id, path, start_line, end_line, kind, metric_value, extra_json)
           VALUES (?, ?, ?, ?, ?, ?, ?)`,
          scanId,
          f.path,
          f.startLine,
          f.endLine,
          f.kind,
          f.metricValue,
          f.extra ? JSON.stringify(f.extra) : null,
        );
      }
      this.pruneOldScans(scan.stream_id, scan.tool, scan.scope);
    });
    this.emitter.emit({
      kind: "completed",
      scanId,
      streamId: scan.stream_id,
      tool: scan.tool,
      scope: scan.scope,
    });
  }

  failScan(scanId: number, errorMessage: string): void {
    const scan = this.requireScan(scanId);
    const completedAt = new Date().toISOString();
    this.stateDb.run(
      `UPDATE code_quality_scan SET status = 'failed', error_message = ?, completed_at = ? WHERE id = ?`,
      errorMessage,
      completedAt,
      scanId,
    );
    this.emitter.emit({
      kind: "failed",
      scanId,
      streamId: scan.stream_id,
      tool: scan.tool,
      scope: scan.scope,
    });
  }

  listScans(query: { streamId: string; limit?: number }): CodeQualityScanRow[] {
    const limit = query.limit ?? 100;
    return this.stateDb.all<CodeQualityScanRow>(
      `SELECT id, stream_id, tool, scope, base_ref, status, error_message, started_at, completed_at
         FROM code_quality_scan
        WHERE stream_id = ?
        ORDER BY started_at DESC, id DESC
        LIMIT ?`,
      query.streamId,
      limit,
    );
  }

  /**
   * Returns findings from the most recent `completed` scan per
   * (stream, tool, scope). When `tool` is omitted, returns findings
   * from the latest completed scan of *each* tool, unioned. `paths`
   * filters to a subset (used by the diff view).
   */
  listLatestFindings(query: ListLatestFindingsQuery): CodeQualityFindingRow[] {
    const tools: CodeQualityTool[] = query.tool ? [query.tool] : ["lizard", "jscpd"];
    const out: CodeQualityFindingRow[] = [];
    for (const tool of tools) {
      const scopes: CodeQualityScope[] = ["codebase", "diff"];
      for (const scope of scopes) {
        const latest = this.stateDb.get<{ id: number }>(
          `SELECT id FROM code_quality_scan
            WHERE stream_id = ? AND tool = ? AND scope = ? AND status = 'completed'
            ORDER BY started_at DESC, id DESC
            LIMIT 1`,
          query.streamId,
          tool,
          scope,
        );
        if (!latest) continue;
        const rows = this.fetchFindingsForScan(latest.id, query.paths);
        out.push(...rows);
      }
    }
    return out;
  }

  private fetchFindingsForScan(scanId: number, paths: string[] | undefined): CodeQualityFindingRow[] {
    if (paths && paths.length === 0) return [];
    let rows: RawFindingRow[];
    if (paths && paths.length > 0) {
      const placeholders = paths.map(() => "?").join(", ");
      rows = this.stateDb.all<RawFindingRow>(
        `SELECT id, scan_id, path, start_line, end_line, kind, metric_value, extra_json
           FROM code_quality_finding
          WHERE scan_id = ? AND path IN (${placeholders})`,
        scanId,
        ...paths,
      );
    } else {
      rows = this.stateDb.all<RawFindingRow>(
        `SELECT id, scan_id, path, start_line, end_line, kind, metric_value, extra_json
           FROM code_quality_finding
          WHERE scan_id = ?`,
        scanId,
      );
    }
    return rows.map((r) => ({
      id: r.id,
      scanId: r.scan_id,
      path: r.path,
      startLine: r.start_line,
      endLine: r.end_line,
      kind: r.kind as CodeQualityFindingKind,
      metricValue: r.metric_value,
      extra: r.extra_json ? (JSON.parse(r.extra_json) as Record<string, unknown>) : null,
    }));
  }

  private requireScan(scanId: number): CodeQualityScanRow {
    const row = this.stateDb.get<CodeQualityScanRow>(
      `SELECT id, stream_id, tool, scope, base_ref, status, error_message, started_at, completed_at
         FROM code_quality_scan WHERE id = ?`,
      scanId,
    );
    if (!row) throw new Error(`code_quality_scan ${scanId} not found`);
    return row;
  }

  private pruneOldScans(streamId: string, tool: CodeQualityTool, scope: CodeQualityScope): void {
    const survivors = this.stateDb.all<{ id: number }>(
      `SELECT id FROM code_quality_scan
        WHERE stream_id = ? AND tool = ? AND scope = ?
        ORDER BY started_at DESC, id DESC
        LIMIT ?`,
      streamId,
      tool,
      scope,
      this.keepLast,
    );
    if (survivors.length === 0) return;
    const placeholders = survivors.map(() => "?").join(", ");
    const ids = survivors.map((r) => r.id);
    this.stateDb.run(
      `DELETE FROM code_quality_finding
        WHERE scan_id IN (
          SELECT id FROM code_quality_scan
           WHERE stream_id = ? AND tool = ? AND scope = ?
             AND id NOT IN (${placeholders})
        )`,
      streamId,
      tool,
      scope,
      ...ids,
    );
    this.stateDb.run(
      `DELETE FROM code_quality_scan
        WHERE stream_id = ? AND tool = ? AND scope = ?
          AND id NOT IN (${placeholders})`,
      streamId,
      tool,
      scope,
      ...ids,
    );
  }
}
