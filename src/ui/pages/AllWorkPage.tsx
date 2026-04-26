import type { ComponentProps } from "react";
import { Page } from "../tabs/Page.js";
import { PlanPane } from "../components/Plan/PlanPane.js";

export type AllWorkPageProps = ComponentProps<typeof PlanPane>;

/**
 * Thin Page wrapper around the existing PlanPane (the per-thread Work
 * panel). The legacy left-rail "Work" tool window stays available during
 * the migration.
 */
export function AllWorkPage(props: AllWorkPageProps) {
  return (
    <Page testId="page-all-work" title="All work" kind="work">
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <PlanPane {...props} />
      </div>
    </Page>
  );
}
