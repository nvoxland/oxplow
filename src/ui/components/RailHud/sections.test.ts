import { describe, expect, test } from "bun:test";
import type { ThreadWorkState, WorkItem } from "../../api.js";
import { computeActiveItem, computePagesDirectory, computeUpNext, type RecentFileEntry, sortRecentFiles } from "./sections.js";
import { gitDashboardRef, uncommittedChangesRef } from "../../tabs/pageRefs.js";

function makeItem(partial: Partial<WorkItem> & { id: string; status: WorkItem["status"] }): WorkItem {
  const base: WorkItem = {
    id: partial.id,
    thread_id: "t-1",
    parent_id: null,
    kind: "task",
    title: partial.id,
    description: "",
    acceptance_criteria: null,
    status: partial.status,
    priority: "medium",
    sort_index: 0,
    created_by: "user",
    created_at: "2026-04-01",
    updated_at: "2026-04-01",
    completed_at: null,
    note_count: 0,
    author: "user",
  };
  return { ...base, ...partial };
}

const baseState = (items: WorkItem[]): ThreadWorkState => ({
  threadId: "t-1",
  waiting: items.filter((i) => i.status === "ready"),
  inProgress: items.filter((i) => i.status === "in_progress"),
  done: items.filter((i) => i.status === "done"),
  epics: [],
  items,
  followups: [],
});

describe("computeActiveItem", () => {
  test("picks the lowest-sort_index in_progress item", () => {
    const a = makeItem({ id: "wi-1", status: "in_progress", sort_index: 5 });
    const b = makeItem({ id: "wi-2", status: "in_progress", sort_index: 2 });
    const state = baseState([a, b]);
    expect(computeActiveItem(state)?.id).toBe("wi-2");
  });

  test("returns null when no in_progress items", () => {
    const state = baseState([makeItem({ id: "wi-1", status: "ready" })]);
    expect(computeActiveItem(state)).toBeNull();
  });

  test("ignores epics for active-item picking", () => {
    const epic = makeItem({ id: "wi-epic", status: "in_progress", sort_index: 1, kind: "epic" });
    const task = makeItem({ id: "wi-task", status: "in_progress", sort_index: 5 });
    const state: ThreadWorkState = {
      threadId: "t-1",
      waiting: [],
      inProgress: [epic, task],
      done: [],
      epics: [epic],
      items: [epic, task],
      followups: [],
    };
    expect(computeActiveItem(state)?.id).toBe("wi-task");
  });

  test("returns null for null state", () => {
    expect(computeActiveItem(null)).toBeNull();
  });

});

describe("computeUpNext", () => {
  test("returns ready items sorted by sort_index", () => {
    const a = makeItem({ id: "wi-3", status: "ready", sort_index: 10 });
    const b = makeItem({ id: "wi-4", status: "ready", sort_index: 1 });
    const c = makeItem({ id: "wi-5", status: "ready", sort_index: 5 });
    const state = baseState([a, b, c]);
    expect(computeUpNext(state).map((i) => i.id)).toEqual(["wi-4", "wi-5", "wi-3"]);
  });

  test("limits result to the requested count", () => {
    const items = [1, 2, 3, 4, 5, 6, 7].map((n) =>
      makeItem({ id: `wi-${n}`, status: "ready", sort_index: n }),
    );
    const state = baseState(items);
    expect(computeUpNext(state, 3).length).toBe(3);
  });

  test("excludes items with non-ready status", () => {
    const a = makeItem({ id: "wi-a", status: "ready", sort_index: 1 });
    const b = makeItem({ id: "wi-b", status: "in_progress", sort_index: 2 });
    const state = baseState([a, b]);
    expect(computeUpNext(state).map((i) => i.id)).toEqual(["wi-a"]);
  });
});

describe("computePagesDirectory", () => {
  test("includes Git dashboard and Uncommitted entries with the canonical refs", () => {
    const entries = computePagesDirectory({ backlogReadyCount: 0 });
    const dash = entries.find((e) => e.id === "git-dashboard");
    const uncommitted = entries.find((e) => e.id === "uncommitted-changes");
    expect(dash?.ref).toEqual(gitDashboardRef());
    expect(uncommitted?.ref).toEqual(uncommittedChangesRef());
  });

  test("Git dashboard appears above Uncommitted", () => {
    const entries = computePagesDirectory({ backlogReadyCount: 0 });
    const ids = entries.map((e) => e.id);
    expect(ids.indexOf("git-dashboard")).toBeLessThan(ids.indexOf("uncommitted-changes"));
  });

  test("Git history is not a top-level rail entry", () => {
    const entries = computePagesDirectory({ backlogReadyCount: 0 });
    expect(entries.find((e) => e.id === "git-history")).toBeUndefined();
  });

  test("Backlog badge surfaces only when backlogReadyCount is > 0", () => {
    expect(computePagesDirectory({ backlogReadyCount: 0 }).find((e) => e.id === "backlog")?.badge).toBeUndefined();
    expect(computePagesDirectory({ backlogReadyCount: 3 }).find((e) => e.id === "backlog")?.badge).toBe(3);
  });

  test("includes the four work pages in plan→done→backlog→archived order", () => {
    const entries = computePagesDirectory({ backlogReadyCount: 0 });
    const ids = entries.map((e) => e.id);
    expect(ids.indexOf("plan-work")).toBeGreaterThanOrEqual(0);
    expect(ids.indexOf("plan-work")).toBeLessThan(ids.indexOf("done-work"));
    expect(ids.indexOf("done-work")).toBeLessThan(ids.indexOf("backlog"));
    expect(ids.indexOf("backlog")).toBeLessThan(ids.indexOf("archived"));
  });
});

describe("sortRecentFiles", () => {
  test("orders by descending touchedAt", () => {
    const entries: RecentFileEntry[] = [
      { path: "src/a.ts", touchedAt: 100 },
      { path: "src/b.ts", touchedAt: 300 },
      { path: "src/c.ts", touchedAt: 200 },
    ];
    expect(sortRecentFiles(entries).map((e) => e.path)).toEqual([
      "src/b.ts",
      "src/c.ts",
      "src/a.ts",
    ]);
  });

  test("limit truncates results", () => {
    const entries: RecentFileEntry[] = Array.from({ length: 10 }, (_, i) => ({
      path: `f${i}.ts`,
      touchedAt: i,
    }));
    expect(sortRecentFiles(entries, 3).length).toBe(3);
  });
});
