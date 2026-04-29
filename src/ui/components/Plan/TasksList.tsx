import { useEffect, useMemo, useState, type ComponentProps } from "react";
import type { ThreadWorkState, WorkItemStatus } from "../../api.js";
import { PlanPane } from "./PlanPane.js";
import {
  TasksFilterBar,
  applyTasksFilters,
  loadTasksFilters,
  saveTasksFilters,
  type TasksFilters,
} from "./TasksFilterBar.js";

type PlanPaneProps = ComponentProps<typeof PlanPane>;

/**
 * Composed list shell for the Tasks page: filter bar above + PlanPane
 * below. Holds the filter state, persists it to localStorage, and
 * applies search/priority filters by preprocessing `threadWork.items`
 * before handing them to PlanPane. Status filter and `showClosed`
 * toggle drive PlanPane's existing `onlyStatuses` / `excludeStatuses`
 * props so the PlanPane internals don't need to know about the
 * filter bar.
 *
 * Owns state that the previous PlanPane-only shell hid behind a
 * kebab toggle (hide auto-filed) so the user-visible knobs sit in
 * one place.
 */
export function TasksList(props: Omit<PlanPaneProps, "hideAuto" | "onlyStatuses" | "excludeStatuses">) {
  const [filters, setFiltersState] = useState<TasksFilters>(() => loadTasksFilters());
  useEffect(() => { saveTasksFilters(filters); }, [filters]);
  const setFilters = (next: TasksFilters) => setFiltersState(next);

  const filteredThreadWork = useMemo<ThreadWorkState | null>(() => {
    if (!props.threadWork) return null;
    if (filters.search.trim().length === 0 && filters.priorities.size === 0) {
      return props.threadWork;
    }
    const filteredItems = applyTasksFilters(props.threadWork.items, filters);
    const allowedIds = new Set(filteredItems.map((i) => i.id));
    return {
      ...props.threadWork,
      items: filteredItems,
      waiting: props.threadWork.waiting.filter((i) => allowedIds.has(i.id)),
      inProgress: props.threadWork.inProgress.filter((i) => allowedIds.has(i.id)),
      done: props.threadWork.done.filter((i) => allowedIds.has(i.id)),
      epics: props.threadWork.epics.filter((i) => allowedIds.has(i.id)),
    };
  }, [props.threadWork, filters]);

  const onlyStatuses: WorkItemStatus[] | undefined = filters.statuses.size > 0
    ? [...filters.statuses]
    : undefined;
  const excludeStatuses: WorkItemStatus[] | undefined = filters.showClosed
    ? undefined
    : ["done", "canceled", "archived"];

  // visibleSections in props takes precedence — Tasks page passes
  // ["toDo", "blocked", "done"] and we don't override.
  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <TasksFilterBar filters={filters} onChange={setFilters} />
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <PlanPane
          {...props}
          threadWork={filteredThreadWork}
          hideAuto={filters.hideAuto}
          onlyStatuses={onlyStatuses}
          excludeStatuses={excludeStatuses}
        />
      </div>
    </div>
  );
}
