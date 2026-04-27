import type { ComponentProps } from "react";
import { Page } from "../tabs/Page.js";
import { PlanPane } from "../components/Plan/PlanPane.js";

export type BacklogPageProps =
  Omit<
    ComponentProps<typeof PlanPane>,
    | "hideAuto"
    | "forceMode"
    | "hideBacklogChip"
    | "visibleSections"
    | "sectionItemLimit"
    | "sectionLabelOverrides"
    | "extraSectionLinks"
    | "excludeStatuses"
    | "onlyStatuses"
    | "hideArchiveToggle"
  >;

/**
 * Full-pane stream-global backlog. Was previously reachable only via
 * the bottom-bar chip toggle inside AllWorkPage; now it's a
 * first-class page so the rail can link directly to it and the badge
 * count belongs to a real destination.
 */
export function BacklogPage(props: BacklogPageProps) {
  return (
    <Page testId="page-backlog" title="Backlog">
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <PlanPane
          {...props}
          forceMode="backlog"
          hideBacklogChip
          hideArchiveToggle
        />
      </div>
    </Page>
  );
}
