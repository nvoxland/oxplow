import type { ReactNode } from "react";
import { useAlerts } from "../components/Alerts/useAlerts.js";
import { useAlertToasts } from "../components/Alerts/useAlertToasts.js";
import { PanelLayoutProvider } from "../components/Panels/PanelLayoutContext.js";
import { PanelRunsProvider } from "../components/Panels/PanelRunsContext.js";
import { ReplacedScreen } from "../components/ReplacedScreen.js";
import { LiveOriginResolver } from "../tabs/liveOrigin.js";
import { current as currentEntry, type OriginItem, type Session } from "../tabs/session.js";
import { useReportUse } from "./useReportUse.js";

export interface WindowProvidersProps {
  streamId: string | null;
  threadId: string | null;
  /** The shown thread's session: each tab opened from a live origin (a
   *  viz node's link column) keeps its list current. */
  session: Session | undefined;
  /** A live origin's pages now, by tab slot (`useWindowTabs` `setLiveItems`). */
  onLiveItems: (slot: string, key: string, items: OriginItem[]) => void;
  /** An alert toast's Review: Alerts, or the page a badge names. */
  onReviewAlerts: (open?: string) => void;
  children: ReactNode;
}

/** What every shell's window sits in: one owner of every extension panel's
 *  runs (read by the rail, the status bar's bell and the Alerts page), the
 *  viewer's left-nav layout, the live origins of the shown thread's tabs,
 *  a toast as each new thing needs the person, and the screen that says
 *  another page load of this device took over. */
export function WindowProviders({ streamId, threadId, session, onLiveItems, onReviewAlerts, children }: WindowProvidersProps) {
  // The daemon sends an agent's call to the device used last.
  useReportUse();
  return (
    <PanelRunsProvider streamId={streamId} threadId={threadId}>
      <PanelLayoutProvider streamId={streamId} threadId={threadId}>
        {(session?.tabs ?? []).map((t) => {
          const live = currentEntry(t).from?.live;
          return live ? (
            <LiveOriginResolver
              key={`${t.slot}/${JSON.stringify(live)}`}
              live={live}
              streamId={streamId}
              onItems={(items) => onLiveItems(t.slot, JSON.stringify(live), items)}
            />
          ) : null;
        })}
        <AlertToasts onReview={onReviewAlerts} />
        {children}
        <ReplacedScreen />
      </PanelLayoutProvider>
    </PanelRunsProvider>
  );
}

/** A toast as each new thing needs the person; Review opens Alerts (a badge's, its page). */
function AlertToasts({ onReview }: { onReview(open?: string): void }) {
  const { items } = useAlerts();
  useAlertToasts(items, onReview);
  return null;
}
