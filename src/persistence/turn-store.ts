import { createId } from "../core/ids.js";
import type { Logger } from "../core/logger.js";
import { getStateDatabase } from "./state-db.js";
import { StoreEmitter } from "./store-emitter.js";

const PROMPT_MAX_LEN = 20_000;
const ANSWER_MAX_LEN = 20_000;

export interface AgentTurn {
  id: string;
  thread_id: string;
  prompt: string;
  answer: string | null;
  session_id: string | null;
  started_at: string;
  ended_at: string | null;
  input_tokens: number | null;
  output_tokens: number | null;
  cache_read_input_tokens: number | null;
  start_snapshot_id: string | null;
  end_snapshot_id: string | null;
}

export interface TurnUsage {
  inputTokens: number | null;
  outputTokens: number | null;
  cacheReadInputTokens: number | null;
}

export type TurnChangeKind = "opened" | "closed";

export interface TurnChange {
  threadId: string;
  turnId: string;
  kind: TurnChangeKind;
}

export interface OpenTurnInput {
  threadId: string;
  prompt: string;
  sessionId?: string | null;
}

export interface CloseTurnInput {
  answer: string | null;
  endedAt?: string;
}

export class TurnStore {
  private readonly stateDb;
  private readonly emitter: StoreEmitter<TurnChange>;

  constructor(projectDir: string, private readonly logger?: Logger) {
    this.stateDb = getStateDatabase(projectDir, logger?.child({ subsystem: "state-db" }));
    this.emitter = new StoreEmitter("turn change", logger);
  }

  subscribe(listener: (change: TurnChange) => void): () => void {
    return this.emitter.subscribe(listener);
  }

  private emit(change: TurnChange): void {
    this.emitter.emit(change);
  }

  openTurn(input: OpenTurnInput): AgentTurn {
    const prompt = clamp(input.prompt, PROMPT_MAX_LEN);
    const now = new Date().toISOString();
    const turn: AgentTurn = {
      id: createId("turn"),
      thread_id: input.threadId,
      prompt,
      answer: null,
      session_id: input.sessionId ?? null,
      started_at: now,
      ended_at: null,
      input_tokens: null,
      output_tokens: null,
      cache_read_input_tokens: null,
      start_snapshot_id: null,
      end_snapshot_id: null,
    };
    this.stateDb.run(
      `INSERT INTO agent_turn (id, thread_id, prompt, answer, session_id, started_at, ended_at)
       VALUES (?, ?, ?, ?, ?, ?, ?)`,
      turn.id,
      turn.thread_id,
      turn.prompt,
      turn.answer,
      turn.session_id,
      turn.started_at,
      turn.ended_at,
    );
    this.emit({ threadId: turn.thread_id, turnId: turn.id, kind: "opened" });
    return turn;
  }

  closeTurn(turnId: string, input: CloseTurnInput): AgentTurn | null {
    const existing = this.getById(turnId);
    if (!existing) return null;
    if (existing.ended_at) return existing;
    const endedAt = input.endedAt ?? new Date().toISOString();
    const answer = input.answer == null ? null : clamp(input.answer, ANSWER_MAX_LEN);
    this.stateDb.run(
      `UPDATE agent_turn SET answer = ?, ended_at = ? WHERE id = ?`,
      answer,
      endedAt,
      turnId,
    );
    const updated = this.getById(turnId);
    if (!updated) return null;
    this.emit({ threadId: updated.thread_id, turnId: updated.id, kind: "closed" });
    return updated;
  }

  setTurnUsage(turnId: string, usage: TurnUsage): AgentTurn | null {
    const existing = this.getById(turnId);
    if (!existing) return null;
    this.stateDb.run(
      `UPDATE agent_turn SET input_tokens = ?, output_tokens = ?, cache_read_input_tokens = ? WHERE id = ?`,
      usage.inputTokens,
      usage.outputTokens,
      usage.cacheReadInputTokens,
      turnId,
    );
    return this.getById(turnId);
  }

  setStartSnapshot(turnId: string, snapshotId: string): void {
    this.stateDb.run(
      `UPDATE agent_turn SET start_snapshot_id = ? WHERE id = ?`,
      snapshotId,
      turnId,
    );
  }

  setEndSnapshot(turnId: string, snapshotId: string): void {
    this.stateDb.run(
      `UPDATE agent_turn SET end_snapshot_id = ? WHERE id = ?`,
      snapshotId,
      turnId,
    );
  }

  currentOpenTurn(threadId: string): AgentTurn | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT * FROM agent_turn WHERE thread_id = ? AND ended_at IS NULL ORDER BY started_at DESC, rowid DESC LIMIT 1`,
      threadId,
    );
    return row ? rowToTurn(row) : null;
  }

  listForThread(threadId: string, limit = 50): AgentTurn[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM agent_turn WHERE thread_id = ? ORDER BY started_at DESC, rowid DESC LIMIT ?`,
        threadId,
        limit,
      )
      .map(rowToTurn);
  }

  getById(turnId: string): AgentTurn | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT * FROM agent_turn WHERE id = ? LIMIT 1`,
      turnId,
    );
    return row ? rowToTurn(row) : null;
  }

  /** Return `cache_read_input_tokens` from the most recently CLOSED turn in
   *  this thread, or null if no turn has closed yet (or the latest closed
   *  turn never had usage recorded). Used by the session-context block to
   *  surface a rough cost signal so the orchestrator can notice when
   *  inline-compounding cache reads should be dispatched to subagents. */
  getLastClosedTurnCacheRead(threadId: string): number | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT cache_read_input_tokens FROM agent_turn
       WHERE thread_id = ? AND ended_at IS NOT NULL
       ORDER BY ended_at DESC, rowid DESC LIMIT 1`,
      threadId,
    );
    if (!row) return null;
    return toNumber(row.cache_read_input_tokens);
  }

  /** Sum `cache_read_input_tokens` across every CLOSED turn in this thread.
   *  Returns 0 when there are no closed turns or every closed turn has a
   *  null value. Used by the Stop-hook directive to emit a fork_thread
   *  hint once the thread's cache-read has accumulated past a threshold. */
  getCumulativeCacheRead(threadId: string): number {
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT COALESCE(SUM(cache_read_input_tokens), 0) AS total FROM agent_turn
       WHERE thread_id = ? AND ended_at IS NOT NULL`,
      threadId,
    );
    if (!row) return 0;
    const total = toNumber(row.total);
    return total ?? 0;
  }
}

function rowToTurn(row: Record<string, unknown>): AgentTurn {
  return {
    id: String(row.id ?? ""),
    thread_id: String(row.thread_id ?? ""),
    prompt: String(row.prompt ?? ""),
    answer: row.answer == null ? null : String(row.answer),
    session_id: row.session_id == null ? null : String(row.session_id),
    started_at: String(row.started_at ?? ""),
    ended_at: row.ended_at == null ? null : String(row.ended_at),
    input_tokens: toNumber(row.input_tokens),
    output_tokens: toNumber(row.output_tokens),
    cache_read_input_tokens: toNumber(row.cache_read_input_tokens),
    start_snapshot_id: row.start_snapshot_id == null ? null : String(row.start_snapshot_id),
    end_snapshot_id: row.end_snapshot_id == null ? null : String(row.end_snapshot_id),
  };
}

function toNumber(raw: unknown): number | null {
  if (raw == null) return null;
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : null;
}

function clamp(raw: string, max: number): string {
  if (raw.length <= max) return raw;
  return `${raw.slice(0, max - 1)}…`;
}
