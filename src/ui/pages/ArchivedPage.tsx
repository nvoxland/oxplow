import type { ComponentProps } from "react";
import { Page } from "../tabs/Page.js";
import { PlanPane } from "../components/Plan/PlanPane.js";

export type ArchivedPageProps =
  Omit<
    ComponentProps<typeof PlanPane>,
    | "hideAuto"
    | "visibleSections"
    | "sectionItemLimit"
    | "sectionLabelOverrides"
    | "extraSectionLinks"
    | "excludeStatuses"
    | "onlyStatuses"
    | "hideBacklogChip"
    | "hideArchiveToggle"
  >;

/**
 * Full descending list of archived items for the current thread.
 * Reached from the Done Work page's "View archived →" link or
 * directly from the rail.
 */
export function ArchivedPage(props: ArchivedPageProps) {
  return (
    <Page testId="page-archived" title="Archived" kind="work">
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <PlanPane
          {...props}
          visibleSections={["done"]}
          onlyStatuses={["archived"]}
          sectionLabelOverrides={{ done: "Archived" }}
          hideBacklogChip
          hideArchiveToggle
        />
      </div>
    </Page>
  );
}
