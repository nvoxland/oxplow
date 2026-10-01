/// Pure view model for Settings → Data (DataSection.tsx): what data the
/// semantic layer holds, who provides it, and how much. See
/// `.context/semantic-layer.md`.
import type { DataEntity, ProjectProgram, ProviderEffect } from "../tauri-bridge/generated/bindings.js";
import { providerChanges } from "./providerEffectText.js";

export interface EntityRowModel {
  name: string;
  owner: string;
  description: string;
  /** Formatted row count, or a status when there is none. */
  rows: string;
  available: boolean;
}

/// One row per entity: core first, then by provider and name. An entity
/// whose source hasn't synced (`declared`) shows "Not synced yet".
export function entityRows(entities: DataEntity[]): EntityRowModel[] {
  const fmt = new Intl.NumberFormat();
  return entities
    .map((e) => {
      const available = e.kind !== "declared";
      return {
        name: e.name,
        owner: e.owner,
        description: e.description,
        rows: !available ? "Not synced yet" : e.rows == null ? "—" : fmt.format(e.rows),
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
/// A shared extension's advisories are shown as what they'd say.
export function programRow(p: ProjectProgram): ProgramRowModel {
  if (p.kind === "advisories") {
    const command = p.args.join("\n");
    return {
      key: `${p.kind}:${p.name}`,
      label: `Advisories from ${p.name}`,
      command,
      status: p.approved ? "Approved on this machine" : "Not approved: they won't reach your agent",
      approved: p.approved,
      approveTitle: `Lets these queries' results into your agent's context (${p.program}). Approve only if you trust this extension; any change needs approval again.`,
    };
  }
  if (p.kind === "provider") {
    // Its grants are part of what's approved: env names, keychain
    // credentials, hosts, and every file of its extension.
    const command = [
      [p.program, ...p.args].join(" "),
      ...(p.env.length > 0 ? [`env: ${p.env.join(", ")}`] : []),
      ...(p.credentials.length > 0 ? [`credentials: ${p.credentials.join(", ")}`] : []),
      ...(p.network.length > 0 ? [`reaches: ${p.network.join(", ")}`] : []),
    ].join("\n");
    return {
      key: `${p.kind}:${p.name}`,
      label: `Provider ${p.name}`,
      command,
      status: p.approved ? "Approved on this machine" : "Not approved: it won't run",
      approved: p.approved,
      approveTitle: `Runs ${p.program} as a long-lived provider with these grants, approving every file in ${p.tree ?? "its extension"} (its declarations included). Approve only if you trust this extension; any change needs approval again.`,
    };
  }
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

/** What approving a provider would change, as lines (P6b.E3): against
 *  what was approved last (`providerChanges`), or everything it declares
 *  at a first approval. */
export function providerEffectLines(e: ProviderEffect): string[] {
  const lines = providerChanges(e).map((l) => l.charAt(0).toUpperCase() + l.slice(1));
  return lines.length > 0 ? lines : ["Nothing changed since it was last approved."];
}

/** A provider's Approve waits until its declaration diff has loaded: a
 *  person approves what they saw change. Other programs approve as listed. */
export function canApprove(p: ProjectProgram, effects: ProviderEffect | "loading" | undefined): boolean {
  return p.kind !== "provider" || (effects !== undefined && effects !== "loading");
}
