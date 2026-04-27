import type { ComponentProps } from "react";
import { Page } from "../tabs/Page.js";
import { PlanPane } from "../components/Plan/PlanPane.js";
import { cardLinkButton } from "../components/Card.js";
import { archivedRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";

export type DoneWorkPageProps =
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
  > & {
    onOpenPage(ref: TabRef): void;
  };

/**
 * Full descending list of done + canceled items for the current
 * thread. Archived items live on the dedicated Archived page (link
 * in the Done section header) so the in-section toggle the legacy
 * AllWorkPage carried is gone.
 */
export function DoneWorkPage({ onOpenPage, ...rest }: DoneWorkPageProps) {
  const viewArchived = (
    <button
      type="button"
      data-testid="done-work-view-archived"
      onClick={(event) => { event.stopPropagation(); onOpenPage(archivedRef()); }}
      style={cardLinkButton}
    >
      View archived →
    </button>
  );
  return (
    <Page testId="page-done-work" title="Done work" kind="work">
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <PlanPane
          {...rest}
          visibleSections={["done"]}
          excludeStatuses={["archived"]}
          extraSectionLinks={{ done: viewArchived }}
          hideBacklogChip
          hideArchiveToggle
        />
      </div>
    </Page>
  );
}
