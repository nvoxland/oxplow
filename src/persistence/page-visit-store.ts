import type { Logger } from "../core/logger.js";
import { getStateDatabase } from "./state-db.js";
import { StoreEmitter } from "./store-emitter.js";

export interface PageVisitInput {
  refKind: string;
  refId: string;
  /** Serializable payload from TabRef.payload. */
  payload: unknown;
  label: string;
  streamId?: string | null;
  threadId?: string | null;
  source?: string | null;
  occurredAt?: string;
}

export interface PageVisit {
  id: number;
  t: string;
  streamId: string | null;
  threadId: string | null;
  refKind: string;
  refId: string;
  payload: unknown;
  label: string;
  source: string | null;
}

export interface PageVisitChange {
  refId: string;
  refKind: string;
  threadId: string | null;
}

export interface ListRecentOpts {
  threadId?: string | null;
  limit: number;
  /** When true, collapses to one row per ref_id (most recent visit wins). */
  dedupeByRef?: boolean;
  /** Filter out specific ref kinds (e.g. ["agent","new-stream"]). */
  excludeKinds?: string[];
}

export interface TopVisitedOpts {
  threadId?: string | null;
  sinceT?: string | null;
  limit: number;
  excludeKinds?: string[];
}

export interface TopVisitedRow {
  refId: string;
  refKind: string;
  payload: unknown;
  label: string;
  count: number;
  lastT: string;
}

export interface CountByDayOpts {
  refId?: string;
  threadId?: string | null;
  sinceT?: string;
  untilT?: string;
}

export interface CountByDayRow {
  day: string;
  count: number;
}

/**
 * Persisted page-visit event log. Append-only; aggregates derived by
 * query. Drives the rail History (`listRecent` with `dedupeByRef`)
 * and longer-term analytics (`topVisited`, `countByDay`).
 *
 * The table stores enough to reconstruct a TabRef without having to
 * cross-reference any other store: refKind + refId + payload_json +
 * the label as it was at visit time.
 */
export class PageVisitStore {
  private readonly stateDb;
  private readonly emitter: StoreEmitter<PageVisitChange>;

  constructor(projectDir: string, logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.emitter = new StoreEmitter("page-visit", logger);
  }

  subscribe(listener: (change: PageVisitChange) => void): () => void {
    return this.emitter.subscribe(listener);
  }

  record(input: PageVisitInput): void {
    const t = input.occurredAt ?? new Date().toISOString();
    const streamId = input.streamId ?? null;
    const threadId = input.threadId ?? null;
    const source = input.source ?? null;
    const payloadJson = JSON.stringify(input.payload ?? null);
    this.stateDb.run(
      `INSERT INTO page_visit (t, stream_id, thread_id, ref_kind, ref_id, payload_json, label, source)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      t,
      streamId,
      threadId,
      input.refKind,
      input.refId,
      payloadJson,
      input.label,
      source,
    );
    this.emitter.emit({ refId: input.refId, refKind: input.refKind, threadId });
  }

  listRecent(opts: ListRecentOpts): PageVisit[] {
    const params: (string | number | null)[] = [];
    const whereThread = scopeClause("thread_id", opts.threadId, params);
    const excludeClause = buildExcludeKinds(opts.excludeKinds, params);
    if (opts.dedupeByRef) {
      params.push(opts.limit);
      const rows = this.stateDb.all<RawRow>(
        `SELECT id, t, stream_id, thread_id, ref_kind, ref_id, payload_json, label, source
           FROM page_visit pv
          WHERE id IN (
            SELECT MAX(id) FROM page_visit
             WHERE 1=1${whereThread}${excludeClause}
             GROUP BY ref_id
          )
          ORDER BY t DESC, id DESC
          LIMIT ?`,
        ...params,
      );
      return rows.map(rowToVisit);
    }
    params.push(opts.limit);
    const rows = this.stateDb.all<RawRow>(
      `SELECT id, t, stream_id, thread_id, ref_kind, ref_id, payload_json, label, source
         FROM page_visit
        WHERE 1=1${whereThread}${excludeClause}
        ORDER BY t DESC, id DESC
        LIMIT ?`,
      ...params,
    );
    return rows.map(rowToVisit);
  }

  topVisited(opts: TopVisitedOpts): TopVisitedRow[] {
    const params: (string | number | null)[] = [];
    const whereThread = scopeClause("thread_id", opts.threadId, params);
    let whereSince = "";
    if (opts.sinceT) {
      whereSince = ` AND t >= ?`;
      params.push(opts.sinceT);
    }
    const excludeClause = buildExcludeKinds(opts.excludeKinds, params);
    params.push(opts.limit);
    const rows = this.stateDb.all<{
      ref_id: string; ref_kind: string; payload_json: string; label: string;
      count: number; last_t: string;
    }>(
      `SELECT pv.ref_id AS ref_id, pv.ref_kind AS ref_kind, pv.payload_json AS payload_json,
              pv.label AS label, agg.count AS count, agg.last_t AS last_t
         FROM page_visit pv
         JOIN (
           SELECT ref_id, COUNT(*) AS count, MAX(t) AS last_t, MAX(id) AS max_id
             FROM page_visit
            WHERE 1=1${whereThread}${whereSince}${excludeClause}
            GROUP BY ref_id
         ) agg ON agg.max_id = pv.id
        ORDER BY agg.count DESC, agg.last_t DESC
        LIMIT ?`,
      ...params,
    );
    return rows.map((r) => ({
      refId: r.ref_id,
      refKind: r.ref_kind,
      payload: parsePayload(r.payload_json),
      label: r.label,
      count: Number(r.count),
      lastT: r.last_t,
    }));
  }

  countByDay(opts: CountByDayOpts): CountByDayRow[] {
    const params: (string | number | null)[] = [];
    let where = "";
    if (opts.refId) {
      where += ` AND ref_id = ?`;
      params.push(opts.refId);
    }
    where += scopeClause("thread_id", opts.threadId, params);
    if (opts.sinceT) {
      where += ` AND t >= ?`;
      params.push(opts.sinceT);
    }
    if (opts.untilT) {
      where += ` AND t < ?`;
      params.push(opts.untilT);
    }
    const rows = this.stateDb.all<{ day: string; count: number }>(
      `SELECT substr(t, 1, 10) AS day, COUNT(*) AS count
         FROM page_visit
        WHERE 1=1${where}
        GROUP BY day
        ORDER BY day ASC`,
      ...params,
    );
    return rows.map((r) => ({ day: r.day, count: Number(r.count) }));
  }

  /** Drop events older than the given ISO timestamp. */
  pruneOlderThan(cutoffIso: string): void {
    this.stateDb.run(`DELETE FROM page_visit WHERE t < ?`, cutoffIso);
  }

  /**
   * Forget every visit for a given page reference. Used when a page
   * is deleted (real persistent page or virtual one like an op-error
   * entry) so it disappears from rail history. Emits a single change
   * event so subscribers refresh.
   */
  forget(refKind: string, refId: string): void {
    this.stateDb.run(
      `DELETE FROM page_visit WHERE ref_kind = ? AND ref_id = ?`,
      refKind,
      refId,
    );
    this.emitter.emit({ refId, refKind, threadId: null });
  }
}

interface RawRow {
  id: number;
  t: string;
  stream_id: string | null;
  thread_id: string | null;
  ref_kind: string;
  ref_id: string;
  payload_json: string;
  label: string;
  source: string | null;
}

function rowToVisit(row: RawRow): PageVisit {
  return {
    id: row.id,
    t: row.t,
    streamId: row.stream_id,
    threadId: row.thread_id,
    refKind: row.ref_kind,
    refId: row.ref_id,
    payload: parsePayload(row.payload_json),
    label: row.label,
    source: row.source,
  };
}

function parsePayload(json: string): unknown {
  try {
    return JSON.parse(json);
  } catch {
    return null;
  }
}

function scopeClause(
  column: string,
  value: string | null | undefined,
  params: (string | null | number)[],
): string {
  if (value === undefined) return "";
  if (value === null) return ` AND ${column} IS NULL`;
  params.push(value);
  return ` AND ${column} = ?`;
}

function buildExcludeKinds(
  kinds: string[] | undefined,
  params: (string | null | number)[],
): string {
  if (!kinds || kinds.length === 0) return "";
  const placeholders = kinds.map(() => "?").join(",");
  for (const k of kinds) params.push(k);
  return ` AND ref_kind NOT IN (${placeholders})`;
}
