import { useState, type ComponentProps } from "react";
import { Page } from "../tabs/Page.js";
import { PlanPane } from "../components/Plan/PlanPane.js";
import { Kebab } from "../components/Kebab.js";
import { cardLinkButton } from "../components/Card.js";
import { backlogRef, doneWorkRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";

export type PlanWorkPageProps =
  Omit<
    ComponentProps<typeof PlanPane>,
    | "hideAuto"
    | "visibleSections"
    | "sectionItemLimit"
    | "sectionLabelOverrides"
    | "extraSectionLinks"
    | "hideBacklogChip"
    | "hideArchiveToggle"
  > & {
    onOpenPage(ref: TabRef): void;
  };

const PREVIEW_LIMIT = 5;

/**
 * Planning surface for the active thread. Shows To Do + Blocked in
 * full, plus the last 5 Done items as previews. The
 * In Progress section is intentionally absent — the rail HUD's
 * "Active item" + "Up next" already surface what the agent is doing
 * right now, so duplicating it here is noise.
 *
 * Header links route to the dedicated Done Work / Backlog pages.
 * The "Hide auto" kebab carries forward the legacy
 * `plan-toggle-hide-auto` filter (suppresses agent-authored rows).
 */
export function PlanWorkPage({ onOpenPage, ...rest }: PlanWorkPageProps) {
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
        {
          id: "plan.view-backlog",
          label: "View backlog →",
          enabled: true,
          run: () => onOpenPage(backlogRef()),
        },
      ]}
    />
  );
  const viewAllDone = (
    <button
      type="button"
      data-testid="plan-work-view-done"
      onClick={(event) => { event.stopPropagation(); onOpenPage(doneWorkRef()); }}
      style={cardLinkButton}
    >
      View all done →
    </button>
  );
  return (
    <Page testId="page-plan-work" title="Plan work" kind="work" actions={actions}>
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <PlanPane
          {...rest}
          hideAuto={hideAuto}
          visibleSections={["toDo", "blocked", "done"]}
          sectionItemLimit={{ done: PREVIEW_LIMIT }}
          extraSectionLinks={{ done: viewAllDone }}
          hideBacklogChip
          hideArchiveToggle
        />
      </div>
    </Page>
  );
}
