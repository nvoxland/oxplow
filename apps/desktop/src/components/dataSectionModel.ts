/// Pure view model for Settings → Data (DataSection.tsx): what data the
/// semantic layer holds, who provides it, and how much. See
/// `.context/semantic-layer.md`.
import type { DataEntity } from "../tauri-bridge/generated/bindings.js";

export interface EntityRowModel {
  name: string;
  owner: string;
  description: string;
  /** Formatted row count, or a status when there is none. */
  rows: string;
  /** Why there's no count, on hover. */
  rowsTitle?: string;
  available: boolean;
}

/** One model's row count as the Data section holds it: absent while it's
 *  still being counted. */
export type EntityCount = { rows: number } | { error: string };

/// One row per entity: core first, then by owner and name. An entity
/// whose source hasn't synced (`declared`) shows "Not synced yet". Counts
/// arrive one model at a time after the list shows (tsk1065): a model too
/// big to count in time reads as a dash with why, not a failed list.
export function entityRows(entities: DataEntity[], counts: Record<string, EntityCount>): EntityRowModel[] {
  const fmt = new Intl.NumberFormat();
  return entities
    .map((e) => {
      const available = e.kind !== "declared";
      const count = counts[e.name];
      return {
        name: e.name,
        owner: e.owner,
        description: e.description,
        rows: !available ? "Not synced yet" : !count ? "Counting…" : "rows" in count ? fmt.format(count.rows) : "—",
        rowsTitle: available && count && "error" in count ? `Couldn't count it: ${count.error}` : undefined,
        available,
      };
    })
    .sort((a, b) => {
      const core = Number(b.owner === "core") - Number(a.owner === "core");
      return core || a.owner.localeCompare(b.owner) || a.name.localeCompare(b.name);
    });
}

/// "12 entities · 3 from extensions" for the section header.
export function entitySummary(rows: EntityRowModel[]): string {
  const ext = rows.filter((r) => r.owner !== "core").length;
  const noun = rows.length === 1 ? "entity" : "entities";
  return ext > 0 ? `${rows.length} ${noun} · ${ext} from extensions` : `${rows.length} ${noun}`;
}

/** What `oxplow.effect.backfill` answers (P9.D5). */
export interface BackfillResult {
  planned: number;
  ran: number;
  skipped: number;
  proposed: number;
  failed: number;
  remaining: number;
  stopped?: string;
}

/** What a person is told before a backfill runs: how many past events the
 *  effect never reacted to (`oxplow.effect.backfill_plan`), and what running it
 *  means. */
export function backfillAsk(effect: string, planned: number): string {
  if (planned === 0) return `${effect} has reacted to every matching event: nothing to backfill.`;
  return planned === 1
    ? `${effect} never reacted to 1 matching event. Backfilling runs it on that event, as it is now; it may call outside oxplow.`
    : `${effect} never reacted to ${planned} matching events. Backfilling runs it on each, oldest first, as it is now; it may call outside oxplow for every one.`;
}

/** A backfill's Run button: one run reacts to at most `batch` events. */
export function backfillRunLabel(planned: number, batch: number): string {
  if (planned > batch) return `Run on the first ${batch} of ${planned}`;
  return `Run on ${planned} ${planned === 1 ? "event" : "events"}`;
}

/** How a backfill went, in a sentence or two. */
export function backfillDone(r: BackfillResult): string {
  const also = [
    r.failed > 0 ? `${r.failed} failed` : null,
    r.proposed > 0 ? `${r.proposed} wait for your approval` : null,
    r.skipped > 0 ? `${r.skipped} skipped` : null,
    !r.stopped && r.remaining > 0 ? `${r.remaining} remain — run it again for the rest` : null,
  ].filter((p): p is string => p !== null);
  const head = `Reacted to ${r.ran} of ${r.planned} ${r.planned === 1 ? "event" : "events"}`;
  const body = also.length > 0 ? `${head}; ${also.join(", ")}.` : `${head}.`;
  return r.stopped ? `${body} Stopped: ${r.stopped}.` : body;
}
