/**
 * Events a pump consumer couldn't take (dead letters, `.context/data-model.md`
 * → "Event pump"): read from `v_event_dead_letter`, retried or discarded by
 * a person through `retry_dead_letter` / `discard_dead_letter`. Settings →
 * Data lists them under Delivery; Alerts shows one row while any wait.
 *
 * Beside them, the reactions of extensions' effects that failed (P9.D4,
 * `v_effect_run`'s latest attempts). One whose every step went to a
 * provider keeping `idempotent_writes` is sent again by itself (P10,
 * `retry_at`); any other — a step outside oxplow may already have run —
 * waits for a person's retry (`effect.retry`, asked first).
 */
import { useCallback, useEffect, useState } from "react";

import { querySql } from "./api.js";
import { recordOpError } from "./components/opErrorsStore.js";
import { NO_READS, useRerunOnChange } from "./lens/lensRerun.js";
import type { Reads, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** One pending dead letter. */
export interface UndeliveredEvent {
  id: number;
  consumer: string;
  eventSeq: number;
  eventType: string;
  error: string;
  attempts: number;
  lastFailedAt: string;
}

export function lettersFromResult(result: SqlQueryResult): UndeliveredEvent[] {
  return result.rows.map((row) => {
    const at = (name: string) => row[result.columns.indexOf(name)] ?? null;
    return {
      id: Number(at("id")),
      consumer: String(at("consumer")),
      eventSeq: Number(at("event_seq")),
      eventType: String(at("event_type")),
      error: String(at("error") ?? ""),
      attempts: Number(at("attempts") ?? 0),
      lastFailedAt: String(at("last_failed_at") ?? ""),
    };
  });
}

/** What a Delivery row says. */
export function letterLine(l: UndeliveredEvent): string {
  return `${l.consumer} couldn't take ${l.eventType} (event ${l.eventSeq}), ${l.attempts === 1 ? "once" : `${l.attempts} times`}`;
}

/** The Alerts row, while any event waits. */
export function deliveryAlert(count: number): string | null {
  if (count === 0) return null;
  return count === 1 ? "1 event couldn't be delivered" : `${count} events couldn't be delivered`;
}

/** The pending dead letters, newest first, and what was read. */
export async function readUndelivered(): Promise<{ letters: UndeliveredEvent[]; reads: Reads }> {
  const res = await querySql(
    `SELECT id, consumer, event_seq, event_type, error, attempts, last_failed_at
       FROM v_event_dead_letter WHERE state = 'pending' ORDER BY id DESC`,
    [],
    1_000,
  );
  return { letters: lettersFromResult(res), reads: res.reads };
}

/** The pending dead letters, re-read when they change. */
export function useUndelivered(): UndeliveredEvent[] {
  const [letters, setLetters] = useState<UndeliveredEvent[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    void readUndelivered()
      .then((r) => {
        setLetters(r.letters);
        setReads(r.reads);
      })
      .catch((e: unknown) => {
        recordOpError({ label: "Read undelivered events", message: e instanceof Error ? e.message : String(e) });
      });
  }, []);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
  return letters;
}

/** An effect's reaction to an event whose latest attempt failed. */
export interface FailedReaction {
  /** `<extension>/<id>`. */
  effect: string;
  eventId: string;
  eventSeq: number;
  /** The attempt that failed, from 1. */
  attempt: number;
  reason: string;
  eventType: string;
  /** When it is sent again by itself (P10); null when it waits for a person. */
  retryAt: string | null;
}

export function reactionsFromResult(result: SqlQueryResult): FailedReaction[] {
  return result.rows.map((row) => {
    const at = (name: string) => row[result.columns.indexOf(name)] ?? null;
    return {
      effect: String(at("effect")),
      eventId: String(at("event_id")),
      eventSeq: Number(at("event_seq")),
      attempt: Number(at("attempt") ?? 1),
      reason: String(at("reason") ?? ""),
      eventType: String(at("event_type") ?? "an event"),
      retryAt: at("retry_at") === null ? null : String(at("retry_at")),
    };
  });
}

/** What a failed reaction's row says. */
export function reactionLine(r: FailedReaction): string {
  const failed = `${r.effect} failed on ${r.eventType} (event ${r.eventSeq})`;
  const line = r.attempt > 1 ? `${failed}, attempt ${r.attempt}` : failed;
  return r.retryAt === null ? line : `${line}; sent again by itself shortly`;
}

/** The reactions whose latest attempt failed, newest first, and what was read. */
export async function readFailedReactions(): Promise<{ reactions: FailedReaction[]; reads: Reads }> {
  const res = await querySql(
    `SELECT r.effect, r.event_id, r.event_seq, r.attempt, r.reason, e.type AS event_type, r.retry_at
       FROM v_effect_run r LEFT JOIN v_event e ON e.id = r.event_id
      WHERE r.latest = 1 AND r.state = 'failed' ORDER BY r.id DESC`,
    [],
    200,
  );
  return { reactions: reactionsFromResult(res), reads: res.reads };
}

/** The failed reactions, re-read when they change. */
export function useFailedReactions(): FailedReaction[] {
  const [reactions, setReactions] = useState<FailedReaction[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    void readFailedReactions()
      .then((r) => {
        setReactions(r.reactions);
        setReads(r.reads);
      })
      .catch((e: unknown) => {
        recordOpError({ label: "Read failed reactions", message: e instanceof Error ? e.message : String(e) });
      });
  }, []);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
  return reactions;
}
