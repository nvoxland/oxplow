import type { WorkspaceStatusSummary } from "../../api.js";
import { statusColor } from "./shared.js";

export function GitSummary({ summary }: { summary: WorkspaceStatusSummary }) {
  return (
    <div style={{ display: "flex", gap: 8, flexWrap: "wrap", color: "var(--muted)", fontSize: 11 }}>
      <span>{summary.total} changed</span>
      {summary.modified > 0 ? <span style={{ color: statusColor("modified") }}>M {summary.modified}</span> : null}
      {summary.added > 0 ? <span style={{ color: statusColor("added") }}>A {summary.added}</span> : null}
      {summary.deleted > 0 ? <span style={{ color: statusColor("deleted") }}>D {summary.deleted}</span> : null}
      {summary.renamed > 0 ? <span style={{ color: statusColor("renamed") }}>R {summary.renamed}</span> : null}
      {summary.untracked > 0 ? <span style={{ color: statusColor("untracked") }}>U {summary.untracked}</span> : null}
    </div>
  );
}
