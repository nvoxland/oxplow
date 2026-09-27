/**
 * The project's compiled zone rules, shared by every surface that paints
 * zones (file-tree badges, the churn treemap + its legend).
 *
 * A module-level cache rather than a prop threaded from the page: the
 * consumers are mounted from four different hosts (TaskDetail, the Files
 * panel, the diff page, the Change-analysis drilldown) and none of them
 * has any other reason to know about zones. The rules are project
 * config, so one fetch serves them all.
 *
 * Refreshes on `configChanged`, which is what `set_zones` emits — an
 * agent rewriting the table repaints the open view without a restart.
 */

import { useSyncExternalStore } from "react";

import { getConfig, subscribeOxplowEvents } from "../../api.js";
import { logUi } from "../../logger.js";
import { compileZoneRules, type CompiledZoneRules } from "./zones.js";

const EMPTY: CompiledZoneRules = compileZoneRules([]);

let snapshot: CompiledZoneRules = EMPTY;
let started = false;
const listeners = new Set<() => void>();

function emit() {
  for (const l of listeners) l();
}

function load() {
  void getConfig()
    .then((cfg) => {
      snapshot = compileZoneRules(cfg.zones ?? []);
      emit();
    })
    .catch((error) => logUi("warn", "failed to load zone rules", { error: String(error) }));
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  if (!started) {
    started = true;
    load();
    subscribeOxplowEvents((event) => {
      if (event.kind === "configChanged") load();
    });
  }
  return () => {
    listeners.delete(listener);
  };
}

export function useZoneRules(): CompiledZoneRules {
  return useSyncExternalStore(
    subscribe,
    () => snapshot,
    () => EMPTY,
  );
}

/** Test seam: set the rules directly and reset the fetch latch. */
export function __setZoneRulesForTest(rules: CompiledZoneRules | null): void {
  snapshot = rules ?? EMPTY;
  started = rules !== null;
  emit();
}
