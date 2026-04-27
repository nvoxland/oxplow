import type { Stream } from "../api.js";
import { Page } from "../tabs/Page.js";
import { HistoryPanel } from "../components/History/HistoryPanel.js";
import type { DiffRequest } from "../components/Diff/diff-request.js";

export interface GitHistoryPageProps {
  stream: Stream | null;
  onOpenDiff?(request: DiffRequest): void;
  revealSha?: { sha: string; token: number } | null;
}

/**
 * Thin Page wrapper around the existing HistoryPanel. Density / polish
 * comes in later phases.
 */
export function GitHistoryPage({ stream, onOpenDiff, revealSha }: GitHistoryPageProps) {
  return (
    <Page testId="page-git-history" title="Git history" kind="commits">
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <HistoryPanel stream={stream} onOpenDiff={onOpenDiff} revealSha={revealSha} />
      </div>
    </Page>
  );
}
