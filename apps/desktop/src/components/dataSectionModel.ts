/// Pure view model for Settings → Data (DataSection.tsx): what data the
/// semantic layer holds, who provides it, and how much. See
/// `.context/semantic-layer.md`.
import type { EntityRowCount, SchemaEntity } from "../tauri-bridge/generated/bindings.js";

export interface EntityRowModel {
  name: string;
  owner: string;
  description: string;
  /** Formatted row count, or a status when there is none. */
  rows: string;
  available: boolean;
}

/// One row per entity: core first, then by provider and name. An entity
/// whose source hasn't synced shows "Not synced yet" instead of a count.
export function entityRows(schema: SchemaEntity[], counts: EntityRowCount[]): EntityRowModel[] {
  const byName = new Map(counts.map((c) => [c.name, c.rows]));
  const fmt = new Intl.NumberFormat();
  return schema
    .map((e) => {
      const n = byName.get(e.name);
      return {
        name: e.name,
        owner: e.owner,
        description: e.description,
        rows: !e.available ? "Not synced yet" : n == null ? "—" : fmt.format(n),
        available: e.available,
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
