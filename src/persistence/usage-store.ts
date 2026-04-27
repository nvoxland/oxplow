import type { Logger } from "../core/logger.js";
import { getStateDatabase } from "./state-db.js";
import { StoreEmitter } from "./store-emitter.js";

export interface UsageRecordInput {
  kind: string;
  key: string;
  event?: string;
  streamId?: string | null;
  threadId?: string | null;
  occurredAt?: string;
}

export interface UsageChange {
  kind: string;
  key: string;
  streamId: string | null;
  threadId: string | null;
}

export interface UsageRollupRow {
  key: string;
  last_at: string;
  count: number;
}

export interface UsageQuery {
  kind: string;
  streamId?: string | null;
  threadId?: string | null;
  limit?: number;
  /** ISO timestamp; rows with `occurred_at < since` are excluded. */
  since?: string;
}

/**
 * Generic (kind, key) usage tracking. Append-only event log; aggregates
 * are derived by query, not stored. Adding a new "kind" (editor file,
 * work item, etc.) needs no schema change. The store coalesces rapid
 * repeat events for the same (kind, key, event) to avoid history spam —
 * if the most recent matching row is younger than `coalesceMs`, its
 * `occurred_at` is bumped instead of inserting a new row.
 */
export class UsageStore {
  private readonly stateDb;
  private readonly emitter: StoreEmitter<UsageChange>;
  private readonly coalesceMs: number;

  constructor(projectDir: string, logger?: Logger, coalesceMs = 30_000) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.emitter = new StoreEmitter("usage", logger);
    this.coalesceMs = coalesceMs;
  }

  subscribe(listener: (change: UsageChange) => void): () => void {
    return this.emitter.subscribe(listener);
  }

  record(input: UsageRecordInput): void {
    const event = input.event ?? "open";
    const occurredAt = input.occurredAt ?? new Date().toISOString();
    const streamId = input.streamId ?? null;
    const threadId = input.threadId ?? null;
    const occurredMs = Date.parse(occurredAt);

    // Coalesce: find the most recent same-(kind, key, event, stream, thread)
    // row; if it's within the window, bump its timestamp.
    const recent = this.stateDb.get<{ id: number; occurred_at: string }>(
      `SELECT id, occurred_at FROM usage_event
       WHERE kind = ? AND key = ? AND event = ?
         AND ((? IS NULL AND stream_id IS NULL) OR stream_id = ?)
         AND ((? IS NULL AND thread_id IS NULL) OR thread_id = ?)
       ORDER BY occurred_at DESC, id DESC
       LIMIT 1`,
      input.kind,
      input.key,
      event,
      streamId,
      streamId,
      threadId,
      threadId,
    );
    if (recent && Number.isFinite(occurredMs)) {
      const recentMs = Date.parse(recent.occurred_at);
      if (Number.isFinite(recentMs) && occurredMs - recentMs < this.coalesceMs) {
        this.stateDb.run(`UPDATE usage_event SET occurred_at = ? WHERE id = ?`, occurredAt, recent.id);
        this.emitter.emit({ kind: input.kind, key: input.key, streamId, threadId });
        return;
      }
    }

    this.stateDb.run(
      `INSERT INTO usage_event (stream_id, thread_id, kind, key, event, occurred_at)
       VALUES (?, ?, ?, ?, ?, ?)`,
      streamId,
      threadId,
      input.kind,
      input.key,
      event,
      occurredAt,
    );
    this.emitter.emit({ kind: input.kind, key: input.key, streamId, threadId });
  }

  /** Most recently accessed keys, aggregated by key. */
  mostRecent(query: UsageQuery): UsageRollupRow[] {
    return this.aggregate(query, "MAX(occurred_at) DESC");
  }

  /** Most frequently accessed keys, aggregated by key. */
  mostFrequent(query: UsageQuery): UsageRollupRow[] {
    return this.aggregate(query, "count DESC, last_at DESC");
  }

  private aggregate(query: UsageQuery, orderBy: string): UsageRollupRow[] {
    const limit = query.limit ?? 20;
    const params: (string | null | number)[] = [query.kind];
    const whereStream = scopeClause("stream_id", query.streamId, params);
    const whereThread = scopeClause("thread_id", query.threadId, params);
    let whereSince = "";
    if (query.since) {
      whereSince = " AND occurred_at >= ?";
      params.push(query.since);
    }
    params.push(limit);
    const rows = this.stateDb.all<{ key: string; last_at: string; count: number }>(
      `SELECT key, MAX(occurred_at) AS last_at, COUNT(*) AS count
         FROM usage_event
        WHERE kind = ?${whereStream}${whereThread}${whereSince}
        GROUP BY key
        ORDER BY ${orderBy}
        LIMIT ?`,
      ...params,
    );
    return rows.map((r) => ({ key: r.key, last_at: r.last_at, count: Number(r.count) }));
  }

  /**
   * Keys whose latest event is `'open'` with no later `'close'`. Returns
   * `[]` for kinds that don't emit close events. Cheap enough as a
   * single grouped query for the cardinalities we expect (dozens, not
   * thousands).
   */
  currentlyOpen(query: { kind: string; streamId?: string | null; threadId?: string | null }): string[] {
    const innerParams: (string | null)[] = [];
    const innerStream = scopeClauseAliased("u2.stream_id", query.streamId, innerParams);
    const innerThread = scopeClauseAliased("u2.thread_id", query.threadId, innerParams);
    const outerParams: (string | null)[] = [query.kind];
    const outerStream = scopeClause("stream_id", query.streamId, outerParams);
    const outerThread = scopeClause("thread_id", query.threadId, outerParams);
    const rows = this.stateDb.all<{ key: string; latest_event: string }>(
      `SELECT key,
              (SELECT event FROM usage_event u2
                WHERE u2.kind = u1.kind AND u2.key = u1.key${innerStream}${innerThread}
                ORDER BY occurred_at DESC, id DESC LIMIT 1) AS latest_event
         FROM usage_event u1
        WHERE kind = ?${outerStream}${outerThread}
        GROUP BY key`,
      ...innerParams,
      ...outerParams,
    );
    return rows.filter((r) => r.latest_event === "open").map((r) => r.key);
  }

  /** Drop events older than the given ISO timestamp. */
  pruneOlderThan(cutoffIso: string): void {
    this.stateDb.run(`DELETE FROM usage_event WHERE occurred_at < ?`, cutoffIso);
  }
}

/** Build ` AND col [IS NULL | = ?]` clause and push a param when applicable. */
function scopeClause(column: string, value: string | null | undefined, params: (string | null | number)[]): string {
  if (value === undefined) return "";
  if (value === null) return ` AND ${column} IS NULL`;
  params.push(value);
  return ` AND ${column} = ?`;
}

function scopeClauseAliased(column: string, value: string | null | undefined, params: (string | null)[]): string {
  if (value === undefined) return "";
  if (value === null) return ` AND ${column} IS NULL`;
  params.push(value);
  return ` AND ${column} = ?`;
}
