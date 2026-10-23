/** Pure row presentation for the collector rows of Settings → Data. */
import type { CollectorListing, Trigger } from "../tauri-bridge/generated/bindings.js";
import { readsChanged } from "../lens/lensRerun.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";

export interface CollectorRowModel {
  id: string;
  /** `manual`, `every <n>m` or `on <event types>`. */
  trigger: string;
  /** "Never run", "Failed", or row counts ("12 pr · 3 review"). */
  status: string;
  lastRunAt: string | null;
  error: string | null;
  /** Unapproved collectors need a person to approve the script first; the
   *  project's own are approved under Programs (they have their own row
   *  there), not by this button. */
  actionLabel: string;
  /** Hover text saying exactly what running it does. */
  actionTitle: string;
  /** Declared credentials and whether each has a value (never the value). */
  credentials: { name: string; set: boolean }[];
  /** "Needs X, Y (set it below)." when some are unset, else null. */
  missingCredentials: string | null;
  /** Why failures turned it off on this machine, else null. */
  disabled: string | null;
  /** A disabled collector can be turned back on. */
  canEnableAgain: boolean;
}

/** What the collector lists read: a collector ran — by hand, on its
 *  schedule or for an event — and each run commits its `collector_run`
 *  row. */
export const COLLECTOR_RUNS: Reads = { models: ["v_collector_run"], tables: [], measures: [] };

/** A collector ran: `v_collector_run` changed. */
export function collectorRan(event: Readonly<Record<string, unknown>>): boolean {
  return readsChanged(event, COLLECTOR_RUNS);
}

/** A collector's trigger as a short phrase. */
export function triggerLabel(t: Trigger): string {
  switch (t.kind) {
    case "manual":
      return "manual";
    case "every":
      return `every ${t.minutes}m`;
    case "on":
      return `on ${t.events.join(", ")}`;
    case "onRun":
      return t.run === "test" ? "after a test run" : "after an analysis run";
  }
}

export function collectorRowModel(l: CollectorListing): CollectorRowModel {
  const st = l.run;
  const status = !st
    ? "Never run"
    : st.status === "error"
      ? "Failed"
      : Object.entries(st.rowCounts)
          .map(([entity, n]) => `${n} ${entity}`)
          .join(" · ") ||
        // A fact collector records facts, not rows (tsk1044).
        (l.spec.facts.length > 0 ? "Recorded facts" : "0 rows");
  const missing = l.credentials.filter((c) => !c.set).map((c) => c.name);
  return {
    id: l.spec.id,
    trigger: triggerLabel(l.spec.trigger),
    status,
    lastRunAt: st?.lastRunAt ?? null,
    error: st?.status === "error" ? (st.error ?? "Unknown error") : null,
    actionLabel: "Sync Now",
    actionTitle: `Run ${l.spec.entry ?? l.spec.id} now`,
    credentials: l.credentials,
    missingCredentials: missing.length > 0 ? `Needs ${missing.join(", ")} (set it under Extensions).` : null,
    disabled: l.disabled ? `Off on this machine: ${l.disabled}` : null,
    canEnableAgain: !!l.disabled,
  };
}
