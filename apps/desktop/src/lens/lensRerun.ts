/// When a lens re-runs (P4.6, `.context/semantic-layer.md` "Subscriptions"):
/// when a model it read changed (`modelsChanged`), when facts landed for a
/// measure its `metric_grid()` read (`metricSamplesChanged`), or when a lens
/// definition changed. A run's `result.reads` says what it read. Every lens
/// host — a lens page, a slot, a dashboard tile, the rail's alerts — uses
/// `useRerunOnChange`, so there is one rule.
import { useEffect, useRef } from "react";

import { subscribeOxplowEvents } from "../api.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";

export const NO_READS: Reads = { models: [], tables: [], measures: [] };

/** What a read of `models` (by their view names) depends on — for a view
 *  loaded through a bespoke IPC that still re-reads on `ModelsChanged`. */
export function readsOf(...models: string[]): Reads {
  return { models, tables: [], measures: [] };
}

/** The extension catalog changed — the daemon's own signal (tsk1030),
 *  sent when a file under `oxplow/extensions/` or the `extensions` config
 *  changed: a lens's definition may have. */
export function lensDefinitionChanged(event: Readonly<Record<string, unknown>>): boolean {
  return event.kind === "extensionsChanged";
}

/** What the enabled extensions contribute — lenses, panels, pages, slot
 *  mounts, prompts — changed: a definition did, or the config did (the
 *  person enabled or disabled an extension). Every host that lists
 *  contributions reloads on this; a single run re-runs on its definition
 *  and its reads (`useRerunOnChange`), not on every config change. */
export function extensionsChanged(event: Readonly<Record<string, unknown>>): boolean {
  return event.kind === "configChanged" || lensDefinitionChanged(event);
}

/** Whether `event` changes something a run read. An empty `measures` list
 *  on a samples event means "unknown", so a metric read re-runs. */
export function readsChanged(
  event: Readonly<Record<string, unknown>>,
  reads: Reads,
): boolean {
  const models = event.models;
  if (event.kind === "modelsChanged" && Array.isArray(models)) {
    return models.some((m) => reads.models.includes(m as string));
  }
  if (event.kind === "metricSamplesChanged" && reads.measures.length > 0) {
    const measures = Array.isArray(event.measures) ? (event.measures as string[]) : [];
    return measures.length === 0 || measures.some((m) => reads.measures.includes(m));
  }
  return false;
}

/** Everything several runs read. */
export function unionReads(list: readonly (Reads | null | undefined)[]): Reads {
  const models = new Set<string>();
  const tables = new Set<string>();
  const measures = new Set<string>();
  for (const r of list) {
    r?.models.forEach((m) => models.add(m));
    r?.tables.forEach((t) => tables.add(t));
    r?.measures.forEach((m) => measures.add(m));
  }
  return { models: [...models], tables: [...tables], measures: [...measures] };
}

/** A burst of commits (a hook's several writes) is one re-run. */
const COALESCE_MS = 100;

/** Re-run `refresh` when an event changes what the last run read, or a
 *  lens definition changed. `reads` and `refresh` may change every render. */
export function useRerunOnChange(reads: Reads, refresh: () => void): void {
  const readsRef = useRef(reads);
  readsRef.current = reads;
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const off = subscribeOxplowEvents((event) => {
      if (!lensDefinitionChanged(event) && !readsChanged(event, readsRef.current)) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        refreshRef.current();
      }, COALESCE_MS);
    });
    return () => {
      if (timer) clearTimeout(timer);
      off();
    };
  }, []);
}
