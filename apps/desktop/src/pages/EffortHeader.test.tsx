import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { closedByLabel, EffortHeader, normalizeWorkItemInput, type EffortHeaderDeps, type EffortRecord } from "./EffortHeader.js";

afterEach(() => cleanup());

/** Deps over a fixed effort row, recording each command run. */
function fake(effort: EffortRecord): { deps: EffortHeaderDeps; ran: Array<[string, unknown]> } {
  const ran: Array<[string, unknown]> = [];
  return {
    ran,
    deps: {
      readEffort: async () => effort,
      runCommand: async (name, input) => {
        ran.push([name, input]);
        return {};
      },
    },
  };
}

const open: EffortRecord = { title: "Fix the parser", workItem: null, endedAt: null, closedBy: null };

test("renaming the effort runs effort.update; clearing it restores the default", async () => {
  const { deps, ran } = fake(open);
  const view = render(<EffortHeader effortId="eff12" onOpenPage={() => {}} deps={deps} />);
  const title = await waitFor(() => view.getByText("Fix the parser"));
  fireEvent.click(title);
  const input = view.getByTestId("effort-title") as HTMLInputElement;
  fireEvent.change(input, { target: { value: "Rewrite the parser" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() => expect(ran).toEqual([["oxplow.effort.update", { effort: "effort:eff12", title: "Rewrite the parser" }]]));

  fireEvent.click(await waitFor(() => view.getByText("Fix the parser")));
  const again = view.getByTestId("effort-title") as HTMLInputElement;
  fireEvent.change(again, { target: { value: "" } });
  fireEvent.keyDown(again, { key: "Enter" });
  await waitFor(() => expect(ran[1]).toEqual(["oxplow.effort.update", { effort: "effort:eff12", title: null }]));
});

test("linking takes a work item; Escape cancels", async () => {
  const { deps, ran } = fake(open);
  const view = render(<EffortHeader effortId="eff12" onOpenPage={() => {}} deps={deps} />);
  await waitFor(() => view.getByTestId("effort-not-linked"));
  fireEvent.click(view.getByTestId("effort-link"));
  const field = view.getByTestId("effort-link-prompt-item");
  fireEvent.keyDown(field, { key: "Escape" });
  expect(view.queryByTestId("effort-link-prompt-item")).toBeNull();

  fireEvent.click(view.getByTestId("effort-link"));
  const again = view.getByTestId("effort-link-prompt-item");
  fireEvent.change(again, { target: { value: "work_item:oxplow:tsk42" } });
  fireEvent.click(view.getByTestId("effort-link-prompt-submit"));
  await waitFor(() =>
    expect(ran).toEqual([["oxplow.effort.link", { effort: "effort:eff12", work_item: "work_item:oxplow:tsk42" }]]),
  );
  await waitFor(() => expect(view.queryByTestId("effort-link-prompt-item") === null).toBe(true));
});

test("a linked effort shows its item and unlinks", async () => {
  const { deps, ran } = fake({ ...open, workItem: "work_item:oxplow:tsk7" });
  const opened: unknown[] = [];
  const view = render(<EffortHeader effortId="eff12" onOpenPage={(r) => opened.push(r)} deps={deps} />);
  fireEvent.click(await waitFor(() => view.getByTestId("effort-linked-item")));
  expect(opened).toHaveLength(1);
  fireEvent.click(view.getByTestId("effort-unlink"));
  await waitFor(() => expect(ran).toEqual([["oxplow.effort.link", { effort: "effort:eff12", work_item: null }]]));
});

test("an open effort closes; a closed one says how it closed", async () => {
  const { deps, ran } = fake(open);
  const view = render(<EffortHeader effortId="eff12" onOpenPage={() => {}} deps={deps} />);
  fireEvent.click(await waitFor(() => view.getByTestId("effort-close")));
  await waitFor(() => expect(ran).toEqual([["oxplow.effort.close", { effort: "effort:eff12" }]]));
  cleanup();

  const closed = fake({ ...open, endedAt: "2026-10-06T00:00:00Z", closedBy: "commit" });
  const after = render(<EffortHeader effortId="eff12" onOpenPage={() => {}} deps={closed.deps} />);
  expect((await waitFor(() => after.getByTestId("effort-closed-by"))).textContent).toBe("Closed by a commit");
  expect(after.queryByTestId("effort-close")).toBeNull();
});

test("a typed item is the active list's id or a work item ref", () => {
  const ids = (id: string) => (/^tsk\d+$/.test(id) ? `work_item:oxplow:${id}` : null);
  expect(normalizeWorkItemInput(" tsk42 ", ids)).toBe("work_item:oxplow:tsk42");
  expect(normalizeWorkItemInput("work_item:issues:ENG-12", ids)).toBe("work_item:issues:ENG-12");
  expect(normalizeWorkItemInput("fix it", ids)).toBeNull();
  expect(normalizeWorkItemInput("tsk42", () => null)).toBeNull();
  expect(closedByLabel("switch")).toBe("Closed when the thread moved on");
});
