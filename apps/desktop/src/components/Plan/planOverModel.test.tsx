import { workItemUri } from "../../uri/uri.js";
import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import type { SqlQueryResult } from "../../tauri-bridge/generated/bindings.js";
import { bucketWorkList, itemsFromResult } from "../../workItems.js";
import { TaskGroupList } from "./TaskGroupList.js";
import { buildGroups } from "./plan-utils.js";

afterEach(cleanup);

// The list renders from the work-item interface — `v_work_item` rows read
// by the data layer, bucketed into the thread's list, grouped by the
// plan's own grouping — whichever list is active.
test("the list renders a thread's work read from v_work_item", () => {
  const result = {
    columns: ["uri", "provider", "title", "body", "state", "parent_uri", "thread_id", "rank", "closed_at", "created_at", "updated_at", "native", "comment_count"],
    rows: [
      ["oxplow:oxplow/work_item?id=E-1&provider=issues", "issues", "Epic", "", "todo", null, 1, 0, null, "t", "t", null, 0],
      ["oxplow:oxplow/work_item?id=E-2&provider=issues", "issues", "Child step", "", "in_progress", "oxplow:oxplow/work_item?id=E-1&provider=issues", 1, 1, null, "t", "t", null, 1],
      ["oxplow:oxplow/work_item?id=E-3&provider=issues", "issues", "Loose item", "", "todo", null, 1, 2, null, "t", "t", null, 0],
    ],
    truncated: false,
    reads: { models: ["v_work_item"], tables: [], measures: [] },
    freshness: {},
  } as unknown as SqlQueryResult;
  const list = bucketWorkList("thr1", itemsFromResult(result), result.reads);
  const groups = buildGroups(list);
  const NOOP = async () => {};
  const view = render(
    <>
      {groups.map((group, i) => (
        <TaskGroupList
          key={i}
          group={group}
          scopeThreadId="thr1"
          onUpdateTask={NOOP}
          onReorderTasks={NOOP}
          onOpenMenu={() => {}}
          epicChildrenMap={new Map(groups.flatMap((g) => [...g.epicChildren.entries()]))}
          onReparentTask={NOOP}
          isSectionCollapsed={() => false}
          onToggleSectionCollapsed={() => {}}
          visibleSections={["inProgress", "ready", "blocked", "done"]}
          fields={[]}
        />
      ))}
    </>,
  );
  expect(view.container.textContent).toContain("Epic");
  expect(view.container.textContent).toContain("Loose item");
  expect(view.getByTestId(`tasks-row-${workItemUri("issues", "E-3")}`)).not.toBeNull();
});
