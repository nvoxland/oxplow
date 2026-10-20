import { useMemo, type ComponentProps } from "react";
import { bucketWorkList, type WorkList } from "../../workItems.js";
import { PlanPane } from "./PlanPane.js";
import { TasksFilterBar } from "./TasksFilterBar.js";
import type { FieldFilter } from "./plan-utils.js";

type PlanPaneProps = ComponentProps<typeof PlanPane>;

/**
 * Composed list shell for the Tasks page: the declared-field filter bar
 * above + PlanPane below. The chips are the page's (its view params,
 * `tasksView.ts`); this narrows the list to the matching items before
 * handing it to PlanPane (an epic stays while any child matches).
 */
export function TasksList({
  filter,
  onFilterChange,
  ...props
}: Omit<PlanPaneProps, "onlyStates" | "excludeStates"> & { filter: FieldFilter; onFilterChange(filter: FieldFilter): void }) {
  const filteredThreadWork = useMemo<WorkList | null>(() => {
    const list = props.threadWork;
    if (!list) return null;
    const chosen = Object.entries(filter).filter(([name, values]) => values.length > 0 && props.fields.some((f) => f.name === name));
    if (chosen.length === 0) return list;
    const matches = (i: WorkList["all"][number]) =>
      chosen.every(([name, values]) => values.includes(String(i.native[name] ?? "")));
    const parents = new Set(list.all.filter(matches).map((i) => i.parentRef).filter((p): p is string => p !== null));
    return bucketWorkList(list.threadId, list.all.filter((i) => matches(i) || parents.has(i.ref)), list.reads);
  }, [props.threadWork, props.fields, filter]);

  // visibleSections in props takes precedence — Tasks page passes
  // ["ready", "blocked", "done"] and we don't override.
  return (
    <div
      data-tasks-roomy
      style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}
    >
      {/* Tasks-page-only spacing overrides. Promotes the section
          headers from sidebar-density (11px uppercase) to wiki-page
          headings, and gives rows real breathing room. Scoped to
          [data-tasks-roomy] so the same TaskGroupList component still
          renders compactly on Plan Work / Done Work / Archived. */}
      <style>{`
        [data-tasks-roomy] [data-testid^="plan-section-header-"] {
          padding: 24px 24px 12px !important;
          font-size: 18px !important;
          font-weight: 600 !important;
          text-transform: none !important;
          letter-spacing: 0 !important;
          color: var(--text-primary) !important;
          background: transparent !important;
          border-top: none !important;
          border-bottom: 1px solid var(--border-subtle) !important;
          position: static !important;
        }
      `}</style>
      <TasksFilterBar fields={props.fields} filter={filter} onChange={onFilterChange} />
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", overflow: "auto" }}>
        <PlanPane
          {...props}
          threadWork={filteredThreadWork}
          excludeStates={["canceled"]}
        />
      </div>
    </div>
  );
}
