import type { Stream } from "../api.js";
import { Page } from "../tabs/Page.js";
import { SnapshotsPanel } from "../components/Snapshots/SnapshotsPanel.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";

export interface LocalHistoryPageProps {
  stream: Stream | null;
  onOpenDiff?(spec: DiffSpec): void;
  revealSnapshotId?: { snapshotId: string; token: number } | null;
  onRequestEditWorkItem?(itemId: string): void;
}

/**
 * Thin Page wrapper around the existing SnapshotsPanel. The panel keeps
 * its current UI; this exists so the rail HUD can route to "Local
 * history" as a full-area tab. Density / web-style polish lands later.
 */
export function LocalHistoryPage({ stream, onOpenDiff, revealSnapshotId, onRequestEditWorkItem }: LocalHistoryPageProps) {
  return (
    <Page testId="page-local-history" title="Local history">
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <SnapshotsPanel
          stream={stream}
          onOpenDiff={onOpenDiff}
          revealSnapshotId={revealSnapshotId}
          onRequestEditWorkItem={onRequestEditWorkItem}
        />
      </div>
    </Page>
  );
}
