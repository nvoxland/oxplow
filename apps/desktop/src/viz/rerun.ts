/// When a viz re-runs (P4.6, `.context/semantic-layer.md` "Subscriptions"):
/// when a dataset it read changed (`datasetsChanged`), when facts landed for a
/// measure its `metric_grid()` read (`metricSamplesChanged`), when a viz
/// definition changed, or after a reconnect (anything may have changed while
/// the socket was down). A run's `result.reads` says what it read. Every viz
/// host — a viz page, a slot, a dashboard tile, the rail's alerts — uses
/// `useRerunOnChange`, so there is one rule.
import { useEffect, useRef, useState } from "react";

import { onRemoteReconnect, subscribeOxplowEvents } from "../api.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";

export const NO_READS: Reads = { datasets: [], tables: [], measures: [] };

/** What a read of `datasets` (by their view names) depends on — for a view
 *  loaded through a bespoke IPC that still re-reads on `DatasetsChanged`. */
export function readsOf(...datasets: string[]): Reads {
  return { datasets, tables: [], measures: [] };
}

/** The extension catalog changed — the daemon's own signal (tsk1030),
 *  sent when an extension was deployed, rolled back, turned on or off, or
 *  removed: a viz's definition may have. */
export function pageDefinitionChanged(event: Readonly<Record<string, unknown>>): boolean {
  return event.kind === "extensionsChanged";
}

/** What the enabled extensions contribute — viz nodes, panels, pages, slot
 *  mounts, prompts — changed: a definition did, or the config did (the
 *  person enabled or disabled an extension). Every host that lists
 *  contributions reloads on this; a single run re-runs on its definition
 *  and its reads (`useRerunOnChange`), not on every config change. */
export function extensionsChanged(event: Readonly<Record<string, unknown>>): boolean {
  return event.kind === "configChanged" || pageDefinitionChanged(event);
}

/** Whether `event` changes something a run read. An empty `measures` list
 *  on a samples event means "unknown", so a metric read re-runs. */
export function readsChanged(
  event: Readonly<Record<string, unknown>>,
  reads: Reads,
): boolean {
  const datasets = event.datasets;
  if (event.kind === "datasetsChanged" && Array.isArray(datasets)) {
    return datasets.some((m) => reads.datasets.includes(m as string));
  }
  if (event.kind === "metricSamplesChanged" && reads.measures.length > 0) {
    const measures = Array.isArray(event.measures) ? (event.measures as string[]) : [];
    return measures.length === 0 || measures.some((m) => reads.measures.includes(m));
  }
  return false;
}

/** Everything several runs read. */
export function unionReads(list: readonly (Reads | null | undefined)[]): Reads {
  const datasets = new Set<string>();
  const tables = new Set<string>();
  const measures = new Set<string>();
  for (const r of list) {
    r?.datasets.forEach((m) => datasets.add(m));
    r?.tables.forEach((t) => tables.add(t));
    r?.measures.forEach((m) => measures.add(m));
  }
  return { datasets: [...datasets], tables: [...tables], measures: [...measures] };
}

/** A burst of commits (a hook's several writes) is one re-run. */
const COALESCE_MS = 100;

/** `run` once the calls to `schedule` go quiet for {@link COALESCE_MS}:
 *  the one way a reader of change events turns a burst of commits into
 *  one re-read. `cancel` drops a pending run. */
export function coalesce(run: () => void): { schedule(): void; cancel(): void } {
  let timer: ReturnType<typeof setTimeout> | null = null;
  return {
    schedule() {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        run();
      }, COALESCE_MS);
    },
    cancel() {
      if (timer) clearTimeout(timer);
      timer = null;
    },
  };
}

/** How long a change that matched nothing yet is remembered, waiting for the
 *  run in flight to say what it read. */
const MISSED_MS = 30_000;

/** The changes a host heard that matched nothing it read yet, by what
 *  changed: each dataset's and measure's latest time, and the latest samples
 *  event that named no measure (any metric read). Bounded by how many names
 *  change, never by how many events said so — every host hears every event,
 *  and a boot sends a burst of them (kept as a list copied on each event, it
 *  once spun the window's thread for most of an hour). Matches as
 *  {@link readsChanged} does. */
export class MissedChanges {
  private datasets = new Map<string, number>();
  private measures = new Map<string, number>();
  private anyMeasureAt: number | null = null;

  /** Remember what `event` changed, at `at`; a kind no read follows is
   *  nothing to remember. */
  note(event: Readonly<Record<string, unknown>>, at: number): void {
    if (event.kind === "datasetsChanged" && Array.isArray(event.datasets)) {
      for (const d of event.datasets) this.datasets.set(d as string, at);
    } else if (event.kind === "metricSamplesChanged") {
      const measures = Array.isArray(event.measures) ? (event.measures as string[]) : [];
      if (measures.length === 0) this.anyMeasureAt = at;
      for (const m of measures) this.measures.set(m, at);
    }
  }

  /** Forget what changed before `at`: a run started then read it. */
  forgetBefore(at: number): void {
    for (const map of [this.datasets, this.measures]) {
      for (const [name, seen] of map) if (seen < at) map.delete(name);
    }
    if (this.anyMeasureAt !== null && this.anyMeasureAt < at) this.anyMeasureAt = null;
  }

  /** Whether a change remembered within the window, at `now`, is one
   *  `reads` read. */
  matches(reads: Reads, now: number): boolean {
    const recent = (at: number | null | undefined) => at != null && now - at < MISSED_MS;
    if (reads.datasets.some((d) => recent(this.datasets.get(d)))) return true;
    if (reads.measures.length === 0) return false;
    return recent(this.anyMeasureAt) || reads.measures.some((m) => recent(this.measures.get(m)));
  }

  /** How many names (and the any-measure mark) are remembered. */
  get size(): number {
    return this.datasets.size + this.measures.size + (this.anyMeasureAt === null ? 0 : 1);
  }
}

/** Re-run `refresh` when an event changes what the last run read, a viz
 *  definition changed, or the daemon is reached again — the events sent
 *  while it was unreachable never arrive (tsk1050). `reads` and `refresh`
 *  may change every render.
 *
 *  A run in flight hasn't said what it read, and a change announced meanwhile
 *  may be one it read the old state of (a write made while the window was
 *  loading). Such events are remembered; when new `reads` arrive and any
 *  remembered event since the last re-run matches them, the host runs once
 *  more. A re-run forgets the events before it started: it read them. */
export function useRerunOnChange(reads: Reads, refresh: () => void): void {
  const readsRef = useRef(reads);
  readsRef.current = reads;
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  const [missed] = useState(() => new MissedChanges());
  const schedule = useRef<() => void>(() => {});
  useEffect(() => {
    const rerun = coalesce(() => {
      missed.forgetBefore(Date.now());
      refreshRef.current();
    });
    schedule.current = rerun.schedule;
    const off = subscribeOxplowEvents((event) => {
      if (pageDefinitionChanged(event) || readsChanged(event, readsRef.current)) {
        rerun.schedule();
        return;
      }
      missed.note(event, Date.now());
    });
    const offReconnect = onRemoteReconnect(rerun.schedule);
    return () => {
      rerun.cancel();
      off();
      offReconnect();
    };
  }, []);
  useEffect(() => {
    if (missed.matches(reads, Date.now())) schedule.current();
  }, [reads]);
}
