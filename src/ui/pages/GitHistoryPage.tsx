import type { Stream } from "../api.js";
import { Page } from "../tabs/Page.js";
import { HistoryPanel } from "../components/History/HistoryPanel.js";
import { gitCommitRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";

export interface GitHistoryPageProps {
  stream: Stream | null;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  revealSha?: { sha: string; token: number } | null;
}

/**
 * Thin Page wrapper around the existing HistoryPanel. Row clicks
 * navigate to the GitCommitPage so commits are first-class
 * bookmark/back/forward citizens.
 */
export function GitHistoryPage({ stream, onOpenPage, revealSha }: GitHistoryPageProps) {
  return (
    <Page testId="page-git-history" title="Git history">
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <HistoryPanel
          stream={stream}
          revealSha={revealSha}
          onSelectCommit={(sha, opts) => onOpenPage(gitCommitRef(sha), opts)}
        />
      </div>
    </Page>
  );
}
