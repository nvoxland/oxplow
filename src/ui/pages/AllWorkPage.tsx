import { useState, type ComponentProps } from "react";
import { Page } from "../tabs/Page.js";
import { PlanPane } from "../components/Plan/PlanPane.js";
import { Kebab } from "../components/Kebab.js";

export type AllWorkPageProps = Omit<ComponentProps<typeof PlanPane>, "hideAuto">;

/**
 * Thin Page wrapper around the existing PlanPane (the per-thread Work
 * panel). The page-header `actions` slot carries a kebab that exposes
 * the legacy `plan-toggle-hide-auto` filter — the original Plan pane's
 * "Hide auto" toggle that suppresses agent-authored rows. Preference is
 * local to this page instance (no DB persistence), matching the pre-
 * redesign behaviour described in `.context/usability.md`.
 */
export function AllWorkPage(props: AllWorkPageProps) {
  const [hideAuto, setHideAuto] = useState(false);
  const actions = (
    <Kebab
      testId="plan-toggle-hide-auto"
      size={16}
      items={[
        {
          id: "plan.hide-auto",
          label: hideAuto ? "Show auto-filed rows" : "Hide auto-filed rows",
          enabled: true,
          run: () => setHideAuto((v) => !v),
        },
      ]}
    />
  );
  return (
    <Page testId="page-all-work" title="All work" kind="work" actions={actions}>
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <PlanPane {...props} hideAuto={hideAuto} />
      </div>
    </Page>
  );
}
