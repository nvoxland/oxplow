import { Page } from "../tabs/Page.js";
import { BottomPanel } from "../components/BottomPanel.js";

export interface HookEventsPageProps {
  /** Stream whose hook events should be rendered. Falls back to a no-data
   *  state when null, since the BottomPanel only subscribes when given an
   *  id. */
  streamId: string | null;
}

/**
 * Page-tab wrapper around the existing `BottomPanel` hook-events view.
 * Phase IA-redesign continuation: the bottom dock is being dissolved into
 * the dashboard, so every panel that lived there needs a Page tab too.
 * Code-quality / Local-history / Git-history already shipped pages; this
 * is the missing one.
 *
 * Stays thin: the data subscription, scroll-to-bottom, and event row
 * rendering all live in `BottomPanel`. The Page wrapper just supplies
 * the shared header chrome so this surface looks like every other Page.
 */
export function HookEventsPage({ streamId }: HookEventsPageProps) {
  return (
    <Page testId="page-hook-events" title="Hook events">
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <BottomPanel streamId={streamId} />
      </div>
    </Page>
  );
}
