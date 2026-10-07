import type { ComponentProps } from "react";
import { Page } from "../tabs/Page.js";
import { PlanPane } from "../components/Plan/PlanPane.js";

export type BacklogPageProps =
  Omit<
    ComponentProps<typeof PlanPane>,
    | "forceMode"
    | "hideBacklogChip"
    | "visibleSections"
    | "sectionItemLimit"
    | "extraSectionLinks"
    | "excludeStates"
    | "onlyStates"
  >;

/**
 * Full-pane backlog: the active list's items on no thread, for a list
 * that keeps items on threads (its `lists` feature).
 */
export function BacklogPage(props: BacklogPageProps) {
  return (
    <Page testId="page-backlog" title="Backlog">
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <PlanPane
          {...props}
          forceMode="backlog"
          hideBacklogChip
        />
      </div>
    </Page>
  );
}
