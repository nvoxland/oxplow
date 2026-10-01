import type { Stream } from "../tauri-bridge/index.js";
import { Page } from "../tabs/Page.js";
import { HistoryPanel } from "../components/History/HistoryPanel.js";
import { gitCommitRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import { LensSlots } from "../lens/LensSlots.js";
import { numericRowId } from "../lens/lensModel.js";
import { useSlotMounted } from "../lens/useSlotMounted.js";

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
  // Extensions' lenses beside the history (`vcs.history.sidebar`), in a
  // side column only when something is mounted there.
  const streamRow = stream ? numericRowId(stream.id) : null;
  const sidebar = useSlotMounted("vcs.history.sidebar", stream?.id ?? null) && streamRow !== null;
  return (
    <Page
      testId="page-git-history"
      title="Git History"
      layout={sidebar ? "details" : "full"}
      rightRail={
        sidebar ? (
          <LensSlots
            slot="vcs.history.sidebar"
            params={{ stream_id: streamRow }}
            streamId={stream?.id ?? null}
            onOpenPage={(ref) => onOpenPage(ref)}
          />
        ) : undefined
      }
    >
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
