/** Pure row presentation for the Settings → Extensions list. */
import type { Extension, SourceListing } from "../tauri-bridge/generated/bindings.js";

export interface ExtensionRowModel {
  name: string;
  description: string;
  lensCount: number;
  /** Where it came from: "In this repo", or `<url> @ <ref> (<sha7>)`. */
  origin: string;
  /** Only git-installed extensions can be updated from their source. */
  canUpdate: boolean;
  healthy: boolean;
  errors: string[];
}

export function extensionRowModel(ext: Extension): ExtensionRowModel {
  const src = ext.source;
  const origin =
    ext.origin === "bundled"
      ? "Ships with oxplow"
      : src
        ? `${src.git}${src.gitRef ? ` @ ${src.gitRef}` : ""} (${src.sha.slice(0, 7)})`
        : "In this repo";
  return {
    name: ext.name,
    description: ext.description,
    lensCount: ext.lenses.length,
    origin,
    canUpdate: src !== null && ext.origin !== "bundled",
    healthy: ext.errors.length === 0,
    errors: ext.errors,
  };
}

export interface SourceRowModel {
  id: string;
  /** `manual` or `every <n>m`. */
  schedule: string;
  /** "Never run", "Failed", or row counts ("12 pr · 3 review"). */
  status: string;
  lastRunAt: string | null;
  error: string | null;
  /** Unapproved sources need a person to approve the script first. */
  action: "approve" | "sync";
  actionLabel: string;
  /** Hover text saying exactly what running it does. */
  actionTitle: string;
}

export function sourceRowModel(l: SourceListing): SourceRowModel {
  const st = l.state;
  const status = !st
    ? "Never run"
    : st.status === "error"
      ? "Failed"
      : Object.entries(st.rowCounts)
          .map(([entity, n]) => `${n} ${entity}`)
          .join(" · ") || "0 rows";
  const env = l.spec.env.length > 0 ? ` with ${l.spec.env.join(", ")} from your environment` : "";
  return {
    id: l.spec.id,
    schedule: l.spec.schedule.kind === "manual" ? "manual" : `every ${l.spec.schedule.minutes}m`,
    status,
    lastRunAt: st?.lastRunAt ?? null,
    error: st?.status === "error" ? (st.error ?? "Unknown error") : null,
    action: l.approved ? "sync" : "approve",
    actionLabel: l.approved ? "Sync Now" : "Approve & Run",
    actionTitle: l.approved
      ? `Run ${l.spec.entry} now`
      : `Runs ${l.extension}/${l.spec.entry} on this machine${env}. Approve only if you trust this extension; a changed script needs approval again.`,
  };
}
