import type { ComponentProps } from "react";
import { Page } from "../tabs/Page.js";
import { PlanPane } from "../components/Plan/PlanPane.js";

export type DoneWorkPageProps = Omit<
  ComponentProps<typeof PlanPane>,
  "visibleSections" | "sectionItemLimit" | "extraSectionLinks" | "excludeStates" | "onlyStates" | "hideBacklogChip"
>;

/** Every closed item (done and canceled) on the current thread's list,
 *  the latest first. */
export function DoneWorkPage(props: DoneWorkPageProps) {
  return (
    <Page testId="page-done-work" title="Done Work">
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <PlanPane {...props} visibleSections={["done"]} hideBacklogChip />
      </div>
    </Page>
  );
}
