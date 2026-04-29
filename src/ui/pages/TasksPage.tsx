import { useState, type ComponentProps } from "react";
import { Page } from "../tabs/Page.js";
import { PlanPane } from "../components/Plan/PlanPane.js";
import { BacklogDrawer } from "../components/Plan/BacklogDrawer.js";
import { Kebab } from "../components/Kebab.js";
import { cardLinkButton } from "../components/Card.js";
import { backlogRef, doneWorkRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";

export type TasksPageProps =
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
    onMoveBacklogItemToThread(itemId: string, toThreadId: string): Promise<void>;
  };

const PREVIEW_LIMIT = 5;

/**
 * Thread-local Tasks page (formerly "Plan work"). Shows To Do +
 * Blocked in full plus the last 5 Done items as previews. The
 * In Progress section is intentionally absent — the rail HUD's
 * "Active item" + "Up next" already surface what the agent is doing
 * right now.
 *
 * The full list + detail split, BacklogDrawer, and inline editing
 * land in follow-up children. This shell keeps the existing PlanPane
 * behaviour while the page rename takes effect so the rest of the
 * app can route to `tasksRef()` immediately.
 */
export function TasksPage({ onOpenPage, onMoveBacklogItemToThread, ...rest }: TasksPageProps) {
  const [hideAuto, setHideAuto] = useState(false);
  const actions = (
    <Kebab
      testId="tasks-kebab"
      size={16}
      items={[
        {
          id: "tasks.hide-auto",
          label: hideAuto ? "Show auto-filed rows" : "Hide auto-filed rows",
          enabled: true,
          run: () => setHideAuto((v) => !v),
        },
        {
          id: "tasks.view-backlog",
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
      data-testid="tasks-view-done"
      onClick={(event) => { event.stopPropagation(); onOpenPage(doneWorkRef()); }}
      style={cardLinkButton}
    >
      View all done →
    </button>
  );
  return (
    <Page testId="page-tasks" title="Tasks" actions={actions}>
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
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
        <BacklogDrawer
          backlog={rest.backlog}
          activeThreadId={rest.activeThreadId}
          onPromote={onMoveBacklogItemToThread}
          onOpenBacklog={() => onOpenPage(backlogRef())}
        />
      </div>
    </Page>
  );
}
