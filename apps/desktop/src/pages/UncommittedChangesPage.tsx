import { LensSlots } from "../lens/LensSlots.js";
import { useChange } from "../lens/useChange.js";
import { useCallback, useState } from "react";
import type { BranchChangeEntry, Stream } from "../api.js";
import { gitCommitAll } from "../api.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { indexRef, opErrorRef, uncommittedChangesRef, type ChangeAnalysisScope } from "../tabs/pageRefs.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { ChangeAnalysisPanel } from "../components/ChangeAnalysis/ChangeAnalysisPanel.js";
import { ScopeFilterBanner } from "../components/ChangeAnalysis/ScopeFilterBanner.js";
import { SummaryCard } from "../components/ChangeAnalysis/SummaryCard.js";
import { useChangeAnalysis } from "../components/ChangeAnalysis/useChangeAnalysis.js";
import {
  summarizeTestFunctions,
  summarizeTestLineRatio,
} from "../components/ChangeAnalysis/analysisHelpers.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";

export interface UncommittedChangesPageProps {
  stream: Stream | null;
  /** Optional drilldown scope. Pivot clicks from inside the embedded
   *  analysis panel set this on the page's own ref so the user
   *  stays here while filtering. */
  scope?: ChangeAnalysisScope;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenDiffInTab?(spec: DiffSpec, siblings?: import("../tabs/PageNavigationContext.js").NavSiblings): void;
}

/**
 * Working-tree page. Renders the standard
 * `SummaryCard` + change-analysis panel pair shared with the commit
 * page; the commit-message form sits between them. The previous
 * custom inline summary + tree-style file picker is gone — the
 * commit button now commits every changed file.
 */
export function UncommittedChangesPage({
  stream,
  scope,
  onOpenPage,
  onOpenFile,
  onOpenDiff,
  onOpenDiffInTab,
}: UncommittedChangesPageProps) {
  const streamId = stream?.id ?? null;
  const { change } = useChange(streamId ? { kind: "working", streamId } : null);
  const analysis = useChangeAnalysis({ streamId, target: "working", scope });
  const [committing, setCommitting] = useState(false);
  const [commitMessage, setCommitMessage] = useState("");

  const fileCount = analysis.files.length;
  const hasUntracked = analysis.files.some((f) => f.status === "untracked");

  const onCommit = useCallback(async () => {
    if (!streamId) return;
    const message = commitMessage.trim();
    if (!message) return;
    if (fileCount === 0) return;
    setCommitting(true);
    try {
      const allPaths = analysis.files.map((f) => f.path);
      const result = await gitCommitAll(streamId, message, {
        paths: allPaths,
        includeUntracked: hasUntracked,
      });
      if (!result.success) {
        const errorId = recordOpError({
          label: "Commit all changes",
          command: `git commit -am "${message.trim()}"`,
          stderr: result.stderr ?? "",
          stdout: result.stdout ?? "",
          exitCode: result.status ?? null,
        });
        onOpenPage(opErrorRef(errorId), { newTab: true });
      } else {
        setCommitMessage("");
        await analysis.refresh();
      }
    } finally {
      setCommitting(false);
    }
  }, [streamId, onOpenPage, commitMessage, fileCount, analysis, hasUntracked]);

  if (!streamId) {
    return (
      <Page testId="page-uncommitted-changes" title="Uncommitted Changes">
        <div style={muted}>No stream selected.</div>
      </Page>
    );
  }

  return (
    <Page testId="page-uncommitted-changes" title="Uncommitted Changes">
      <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: 16, overflow: "auto" }}>
        {scope ? (
          <ScopeFilterBanner
            scope={scope}
            onClear={() => onOpenPage(uncommittedChangesRef())}
          />
        ) : null}
        {analysis.error ? <div style={errorBanner}>{analysis.error}</div> : null}

        {fileCount > 0 ? (
          <SummaryCard
            fileCount={fileCount}
            additions={analysis.totals.additions}
            deletions={analysis.totals.deletions}
            byStatus={analysis.pivots.byStatus}
            tests={analysis.tests}
            testFunctions={summarizeTestFunctions(analysis.functions)}
            testLineRatio={summarizeTestLineRatio(analysis.functionChurn)}
          />
        ) : null}

        {fileCount > 0 ? (
          <section data-testid="uncommitted-commit-form" style={card}>
            <div style={{ fontWeight: 600, marginBottom: 8 }}>Commit</div>
            <textarea
              data-testid="uncommitted-commit-message"
              value={commitMessage}
              onChange={(e) => setCommitMessage(e.target.value)}
              placeholder="Commit message"
              rows={3}
              style={{
                width: "100%",
                boxSizing: "border-box",
                padding: 8,
                fontFamily: "inherit",
                fontSize: "var(--text-sm)",
                border: "1px solid var(--border-subtle)",
                borderRadius: 4,
                background: "var(--surface-input, var(--surface-card))",
                color: "var(--text-primary)",
                resize: "vertical",
              }}
              onKeyDown={(e) => {
                if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                  e.preventDefault();
                  void onCommit();
                }
              }}
            />
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginTop: 8 }}>
              <span style={subtle}>
                {fileCount} file{fileCount === 1 ? "" : "s"} pending
              </span>
              <button
                type="button"
                data-testid="uncommitted-commit-button"
                onClick={onCommit}
                disabled={committing || fileCount === 0 || commitMessage.trim().length === 0}
                style={{
                  ...primaryButton,
                  opacity: committing || fileCount === 0 || commitMessage.trim().length === 0 ? 0.5 : 1,
                  cursor: committing || fileCount === 0 || commitMessage.trim().length === 0 ? "not-allowed" : "pointer",
                }}
              >
                {committing ? "Committing…" : `Commit ${fileCount}`}
              </button>
            </div>
          </section>
        ) : null}

        {fileCount === 0 && !analysis.loading ? (
          <div data-testid="uncommitted-clean" style={cleanState}>
            <span>Working tree is clean.</span>
            <button
              type="button"
              onClick={(e) => onOpenPage(indexRef("git-history"), { newTab: e.metaKey || e.ctrlKey })}
              style={historyLink}
            >
              View git history →
            </button>
          </div>
        ) : (
          <>
            <ChangeAnalysisPanel
              analysis={analysis}
              target="working"
              scope={scope}
              showHeader={false}
              onOpenPage={onOpenPage}
              onOpenFile={onOpenFile}
              onOpenDiff={onOpenDiff}
              onOpenDiffInTab={onOpenDiffInTab}
            />
            <LensSlots
              slot="uncommitted"
              params={change ? { change_id: change.id } : null}
              streamId={streamId}
              onOpenPage={(ref) => onOpenPage(ref)}
            />
          </>
        )}
      </div>
    </Page>
  );
}

/**
 * Status counts for a list of changed files. Kept as a pure helper
 * for the existing test suite even though the page now renders
 * `SummaryCard` (which derives equivalent counts via
 * `analysis.pivots.byStatus`).
 */
export interface SummaryNumbers {
  total: number;
  modified: number;
  added: number;
  deleted: number;
  renamed: number;
  untracked: number;
  additions: number;
  deletions: number;
}

export function summarize(files: BranchChangeEntry[]): SummaryNumbers {
  const out: SummaryNumbers = {
    total: files.length,
    modified: 0,
    added: 0,
    deleted: 0,
    renamed: 0,
    untracked: 0,
    additions: 0,
    deletions: 0,
  };
  for (const file of files) {
    out[file.status] += 1;
    out.additions += file.additions ?? 0;
    out.deletions += file.deletions ?? 0;
  }
  return out;
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-sm)" };
const subtle: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-xs)" };
const errorBanner: React.CSSProperties = {
  padding: 8,
  background: "var(--surface-warning, #fef3c7)",
  color: "var(--text-warning, #92400e)",
  borderRadius: 4,
};
const cleanState: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 16,
  display: "flex",
  alignItems: "center",
  gap: 12,
  fontSize: "var(--text-sm)",
  color: "var(--text-muted)",
};
const historyLink: React.CSSProperties = {
  background: "transparent",
  border: "none",
  padding: 0,
  color: "var(--text-link, #2563eb)",
  cursor: "pointer",
  fontSize: "var(--text-sm)",
};
const primaryButton: React.CSSProperties = {
  padding: "4px 10px",
  background: "var(--surface-action, #2563eb)",
  color: "var(--text-inverse, white)",
  border: "none",
  borderRadius: 4,
  cursor: "pointer",
};
