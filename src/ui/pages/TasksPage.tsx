import { type ComponentProps } from "react";
import { Page } from "../tabs/Page.js";
import { PlanPane } from "../components/Plan/PlanPane.js";
import { TasksList } from "../components/Plan/TasksList.js";
import { TaskDetailPane } from "../components/Plan/TaskDetailPane.js";
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
    | "onlyStatuses"
    | "excludeStatuses"
  > & {
    onOpenPage(ref: TabRef): void;
    onMoveBacklogItemToThread(itemId: string, toThreadId: string): Promise<void>;
  };

const PREVIEW_LIMIT = 5;

/**
 * Thread-local Tasks page (formerly "Plan work"). Composes three
 * pieces:
 *
 * - **TasksList** (left): filter bar + filtered PlanPane. Owns the
 *   filter state (search, status chips, priority chips, hide-auto,
 *   show-closed) persisted to localStorage `tasks-filters`.
 * - **TaskDetailPane** (right, fixed-width 280px): summary view
 *   showing counts, oldest blocked, and recent completions. Always
 *   visible; the rich row editor still opens through the PlanPane
 *   modal flow.
 * - **BacklogDrawer** (bottom, collapsible): the global backlog with
 *   one-click Promote → into the active thread.
 *
 * The In Progress section in PlanPane is intentionally rendered (To Do
 * + Blocked + Done preview) — the rail HUD's "Active item" + "Up
 * next" surfaces a different view of the same data.
 */
export function TasksPage({ onOpenPage, onMoveBacklogItemToThread, ...rest }: TasksPageProps) {
  const actions = (
    <Kebab
      testId="tasks-kebab"
      size={16}
      items={[
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
        <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "row" }}>
          <TasksList
            {...rest}
            visibleSections={["toDo", "blocked", "done"]}
            sectionItemLimit={{ done: PREVIEW_LIMIT }}
            extraSectionLinks={{ done: viewAllDone }}
            hideBacklogChip
            hideArchiveToggle
          />
          <TaskDetailPane threadWork={rest.threadWork} />
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
