/// Pure view model for Settings → Data (DataSection.tsx): what data the
/// semantic layer holds, who provides it, and how much. See
/// `.context/semantic-layer.md`.
import type { EntityRowCount, ProjectProgram, SchemaEntity } from "../tauri-bridge/generated/bindings.js";

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

export interface ProgramRowModel {
  key: string;
  label: string;
  /** What runs, as a command line. */
  command: string;
  status: string;
  approved: boolean;
  /** The Approve button's hover: what consenting means. */
  approveTitle: string;
}

/// A program the project's config would run (an `exec` gauge or collection
/// plugin): unapproved ones don't run until a person approves them here.
export function programRow(p: ProjectProgram): ProgramRowModel {
  const command = [...(p.env ?? []), p.program, ...p.args].join(" ");
  const what = p.kind === "gauge" ? "Gauge" : p.kind === "plugin" ? "Collection plugin" : "ACP agent";
  return {
    key: `${p.kind}:${p.name}`,
    label: `${what} ${p.name}`,
    command,
    status: p.approved ? "Approved on this machine" : "Not approved: it won't run",
    approved: p.approved,
    approveTitle: `Runs ${command} from this project's config on this machine. Approve only if you trust this repo; a changed program or arguments need approval again.`,
  };
}
