/**
 * Events a pump consumer couldn't take (dead letters, `.context/data-model.md`
 * → "Event pump"): read from `v_event_dead_letter`, retried or discarded by
 * a person through `retry_dead_letter` / `discard_dead_letter`. Settings →
 * Data lists them under Delivery; Alerts shows one row while any wait.
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
