import { useMemo } from "react";
import { gitCommitRef, uncommittedChangesRef, type ChangeAnalysisScope } from "../../tabs/pageRefs.js";
import type { TabRef } from "../../tabs/tabState.js";
import type { DiffSpec } from "../Diff/DiffPane.js";
import { ChangeAnalysisHeader } from "./ChangeAnalysisHeader.js";
import { ChangeAnalysisDrilldown } from "./ChangeAnalysisDrilldown.js";
import type { ChangeAnalysisState } from "./useChangeAnalysis.js";

export interface ChangeAnalysisPanelProps {
  /** Result from `useChangeAnalysis`, supplied by the host so a
   *  shared SummaryCard above the panel can read the same state
   *  without double-fetching. */
  analysis: ChangeAnalysisState;
  /** "working" or a commit SHA. Used for empty-state copy and the
   *  back-to-source link. */
  target: string;
  /** Optional drilldown filter. Data is filtered by scope but every
   *  panel still renders. */
  scope?: ChangeAnalysisScope;
  /** Whether to render the ChangeAnalysisHeader (back-to-source +
   *  refresh). Hosts that already have their own header (e.g. the
   *  GitCommitPage) hide it. */
  showHeader?: boolean;
  /** Whether to render the inline Files/Functions panel. The diff view
   *  owns its own top "Files Changed" section (with the same toggle), so
   *  it passes `false` to avoid a duplicate file tree. Default `true`. */
  showFilesPanel?: boolean;
  /** Whether to render the Change Treemap. The diff view hoists it to the
   *  top, so it passes `false`. Default `true`. */
  showTreemap?: boolean;
  /** Whether to render the "Look here first" card. The diff view hoists
   *  it to the top, so it passes `false`. Default `true`. */
  showLookHere?: boolean;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenDiffInTab?(spec: DiffSpec, siblings?: import("../../tabs/PageNavigationContext.js").NavSiblings): void;
}

/**
 * Reusable Change Analysis content. Renders the optional header,
 * the error banner, an empty / loading placeholder, or the full
 * drilldown panel set. Stripped of any `Page` chrome AND of the
 * SummaryCard — hosts render the summary at their own top-of-page
 * position so it sits above any host-specific content (e.g. the
 * commit message form on the uncommitted page).
 */
export function ChangeAnalysisPanel({
  analysis,
  target,
  scope,
  showHeader = true,
  showFilesPanel = true,
  showTreemap = true,
  showLookHere = true,
  onOpenPage,
  onOpenFile,
  onOpenDiff,
  onOpenDiffInTab,
}: ChangeAnalysisPanelProps) {
  const sourceLink = useMemo<TabRef | null>(() => {
    if (target === "working") return uncommittedChangesRef();
    return gitCommitRef(target);
  }, [target]);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      {showHeader ? (
        <ChangeAnalysisHeader
          target={target}
          loading={analysis.loading}
          onRefresh={() => void analysis.refresh()}
          sourceLink={sourceLink}
          onOpenPage={onOpenPage}
        />
      ) : null}

      {analysis.error ? <div style={errorBanner}>{analysis.error}</div> : null}

      {analysis.loading && analysis.files.length === 0 ? (
        <div style={muted}>Loading…</div>
      ) : analysis.files.length === 0 ? (
        <div style={muted}>
          {scope
            ? `No files match ${formatScope(scope)}.`
            : target === "working"
              ? "Working tree is clean."
              : "No file changes in this commit."}
        </div>
      ) : (
        <ChangeAnalysisDrilldown
          scope={scope}
          target={target}
          analysis={analysis}
          showFilesPanel={showFilesPanel}
          showTreemap={showTreemap}
          showLookHere={showLookHere}
          onOpenFile={onOpenFile}
          onOpenDiff={onOpenDiff}
          onOpenDiffInTab={onOpenDiffInTab}
        />
      )}
    </div>
  );
}

function formatScope(scope: ChangeAnalysisScope): string {
  if (scope.kind === "ext") return `.${scope.value} files`;
  if (scope.kind === "dir") return `${scope.value}/`;
  return scope.value;
}

const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-sm)", padding: 16 };
const errorBanner: React.CSSProperties = {
  padding: 8,
  background: "var(--surface-warning, #fef3c7)",
  color: "var(--text-warning, #92400e)",
  borderRadius: 4,
};
