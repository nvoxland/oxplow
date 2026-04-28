import type { WorkspaceStatusSummary } from "../../api.js";
import { FileStatusCountsForSummary } from "../FileStatusCounts.js";

export function GitSummary({ summary }: { summary: WorkspaceStatusSummary }) {
  if (summary.total === 0) {
    return (
      <div style={{ color: "var(--muted)", fontSize: 11, fontStyle: "italic" }}>
        clean
      </div>
    );
  }
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap", color: "var(--muted)", fontSize: 11 }}>
      <span>{summary.total} changed</span>
      <FileStatusCountsForSummary summary={summary} testId="left-panel-git-summary-counts" />
    </div>
  );
}
