import { afterEach, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import type { OpenAgentTurn } from "../../api.js";
import { TaskGroupList } from "./TaskGroupList.js";
import type { TaskGroup } from "./plan-utils.js";

afterEach(cleanup);

const NOOP_ASYNC = async () => {};

function renderList(openTurns: OpenAgentTurn[], group?: TaskGroup) {
  const g: TaskGroup = group ?? { epic: null, items: [], epicChildren: new Map() };
  return render(
    <TaskGroupList
      group={g}
      scopeThreadId="thr1"
      onUpdateTask={NOOP_ASYNC}
      onReorderTasks={NOOP_ASYNC}
      onOpenMenu={() => {}}
      epicChildrenMap={g.epicChildren}
      onReparentTask={NOOP_ASYNC}
      isSectionCollapsed={() => false}
      onToggleSectionCollapsed={() => {}}
      openTurns={openTurns}
      agentStatus="working"
    />,
  );
}

test("open agent turns render as live rows in the In Progress section", () => {
  const { getByTestId } = renderList([
    { id: "atu7", threadId: "thr1", prompt: "refactor the parser", startedAt: "2026-06-12T00:00:00Z" },
  ]);
  const row = getByTestId("plan-live-turn-atu7");
  expect(row.textContent).toContain("refactor the parser");
  // The live row lives inside the In Progress section.
  const section = getByTestId("plan-section-inProgress");
  expect(section.contains(row)).toBe(true);
});

test("live turn rows replace the empty-state placeholder", () => {
  const { queryByText } = renderList([
    { id: "atu7", threadId: "thr1", prompt: "p", startedAt: "2026-06-12T00:00:00Z" },
  ]);
  // With a live row present, neither placeholder variant renders.
  expect(queryByText("Thinking...")).toBeNull();
  expect(queryByText("Waiting")).toBeNull();
});

test("without open turns the placeholder still renders", () => {
  const { getByTestId } = renderList([]);
  const section = getByTestId("plan-section-inProgress");
  expect(section.textContent).toContain("Thinking...");
});
