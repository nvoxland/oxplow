import { createId } from "../core/ids.js";
import type { Logger } from "../core/logger.js";
import { getStateDatabase } from "./state-db.js";
import { StoreEmitter } from "./store-emitter.js";

const PROMPT_MAX_LEN = 20_000;
const ANSWER_MAX_LEN = 20_000;

export interface AgentTurn {
  id: string;
  batch_id: string;
  work_item_id: string | null;
  prompt: string;
  answer: string | null;
  session_id: string | null;
  started_at: string;
  ended_at: string | null;
}

export type TurnChangeKind = "opened" | "closed";

export interface TurnChange {
  batchId: string;
  turnId: string;
  kind: TurnChangeKind;
}

export interface OpenTurnInput {
  batchId: string;
  prompt: string;
  sessionId?: string | null;
}

export interface CloseTurnInput {
  workItemId: string | null;
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
      batch_id: input.batchId,
      work_item_id: null,
      prompt,
      answer: null,
      session_id: input.sessionId ?? null,
      started_at: now,
      ended_at: null,
    };
    this.stateDb.run(
      `INSERT INTO agent_turn (id, batch_id, work_item_id, prompt, answer, session_id, started_at, ended_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      turn.id,
      turn.batch_id,
      turn.work_item_id,
      turn.prompt,
      turn.answer,
      turn.session_id,
      turn.started_at,
      turn.ended_at,
    );
    this.emit({ batchId: turn.batch_id, turnId: turn.id, kind: "opened" });
    return turn;
  }

  closeTurn(turnId: string, input: CloseTurnInput): AgentTurn | null {
    const existing = this.getById(turnId);
    if (!existing) return null;
    if (existing.ended_at) return existing;
    const endedAt = input.endedAt ?? new Date().toISOString();
    const answer = input.answer == null ? null : clamp(input.answer, ANSWER_MAX_LEN);
    this.stateDb.run(
      `UPDATE agent_turn SET work_item_id = ?, answer = ?, ended_at = ? WHERE id = ?`,
      input.workItemId,
      answer,
      endedAt,
      turnId,
    );
    const updated = this.getById(turnId);
    if (!updated) return null;
    this.emit({ batchId: updated.batch_id, turnId: updated.id, kind: "closed" });
    return updated;
  }

  currentOpenTurn(batchId: string): AgentTurn | null {
    const row = this.stateDb.get<Record<string, unknown>>(
      `SELECT * FROM agent_turn WHERE batch_id = ? AND ended_at IS NULL ORDER BY started_at DESC, rowid DESC LIMIT 1`,
      batchId,
    );
    return row ? rowToTurn(row) : null;
  }

  listForBatch(batchId: string, limit = 50): AgentTurn[] {
    return this.stateDb
      .all<Record<string, unknown>>(
        `SELECT * FROM agent_turn WHERE batch_id = ? ORDER BY started_at DESC, rowid DESC LIMIT ?`,
        batchId,
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
}

function rowToTurn(row: Record<string, unknown>): AgentTurn {
  return {
    id: String(row.id ?? ""),
    batch_id: String(row.batch_id ?? ""),
    work_item_id: row.work_item_id == null ? null : String(row.work_item_id),
    prompt: String(row.prompt ?? ""),
    answer: row.answer == null ? null : String(row.answer),
    session_id: row.session_id == null ? null : String(row.session_id),
    started_at: String(row.started_at ?? ""),
    ended_at: row.ended_at == null ? null : String(row.ended_at),
  };
}

function clamp(raw: string, max: number): string {
  if (raw.length <= max) return raw;
  return `${raw.slice(0, max - 1)}…`;
}
