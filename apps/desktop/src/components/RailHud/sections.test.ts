import { describe, expect, test } from "bun:test";
import type { ThreadWorkState, Task } from "../../api.js";
import { computeActiveEpicContext, computeActiveItem, computePagesDirectory, computeUpNext } from "./sections.js";
import { gitDashboardRef, uncommittedChangesRef } from "../../tabs/pageRefs.js";

function makeItem(partial: Partial<Task> & { id: number; status: Task["status"]; kind?: string }): Task {
  // Local `kind` is just a marker used by these test fixtures to flag
  // items that should land in the epics bucket. The Task type itself no
  // longer carries `kind` — we strip it before returning.
  const { kind: _kind, ...rest } = partial;
  const base: Task = {
    id: partial.id,
    thread_id: "t-1",
    parent_id: null,
    title: String(partial.id),
    description: "",
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
  return { ...base, ...rest };
}

const baseState = (items: Task[]): ThreadWorkState => ({
  threadId: "t-1",
  waiting: items.filter((i) => i.status === "ready"),
  inProgress: items.filter((i) => i.status === "in_progress"),
  done: items.filter((i) => i.status === "done"),
  // Treat any item that has at least one child in the list as an epic —
  // the backend uses the same has-children rule.
  epics: items.filter((parent) => items.some((c) => c.parent_id === parent.id)),
  items,
  followups: [],
});

describe("computeActiveItem", () => {
  test("picks the lowest-sort_index in_progress item", () => {
    const a = makeItem({ id: 1, status: "in_progress", sort_index: 5 });
    const b = makeItem({ id: 2, status: "in_progress", sort_index: 2 });
    const state = baseState([a, b]);
    expect(computeActiveItem(state)?.id).toBe(2);
  });

  test("returns null when no in_progress items", () => {
    const state = baseState([makeItem({ id: 1, status: "ready" })]);
    expect(computeActiveItem(state)).toBeNull();
  });

  test("ignores epics for active-item picking", () => {
    const epic = makeItem({ id: 9000, status: "in_progress", sort_index: 1, kind: "epic" });
    const task = makeItem({ id: 9001, status: "in_progress", sort_index: 5 });
    const state: ThreadWorkState = {
      threadId: "t-1",
      waiting: [],
      inProgress: [epic, task],
      done: [],
      epics: [epic],
      items: [epic, task],
      followups: [],
    };
    expect(computeActiveItem(state)?.id).toBe(9001);
  });

  test("returns null for null state", () => {
    expect(computeActiveItem(null)).toBeNull();
  });

});

describe("computeActiveEpicContext", () => {
  test("returns the parent epic and its children sorted by sort_index", () => {
    const epic = makeItem({ id: 9000, kind: "epic", status: "in_progress", sort_index: 0 });
    const c1 = makeItem({ id: 9101, parent_id: 9000, status: "done", sort_index: 1 });
    const c2 = makeItem({ id: 9102, parent_id: 9000, status: "in_progress", sort_index: 2 });
    const c3 = makeItem({ id: 9103, parent_id: 9000, status: "ready", sort_index: 3 });
    const state = baseState([epic, c2, c1, c3]);
    const ctx = computeActiveEpicContext(state, c2);
    expect(ctx?.epic.id).toBe(9000);
    expect(ctx?.children.map((c) => c.id)).toEqual([9101, 9102, 9103]);
  });

  test("returns null when active item has no parent", () => {
    const t = makeItem({ id: 9200, status: "in_progress" });
    expect(computeActiveEpicContext(baseState([t]), t)).toBeNull();
  });

  test("returns null when parent does not exist in the state", () => {
    // The kind discriminator is gone — any parent task is treated as
    // an epic anchor. The only path to null (other than no parent_id)
    // is when the parent isn't loaded into state.
    const child = makeItem({ id: 9301, parent_id: 9300, status: "in_progress" });
    expect(computeActiveEpicContext(baseState([child]), child)).toBeNull();
  });
});

describe("computeUpNext", () => {
  test("returns ready items sorted by sort_index", () => {
    const a = makeItem({ id: 3, status: "ready", sort_index: 10 });
    const b = makeItem({ id: 4, status: "ready", sort_index: 1 });
    const c = makeItem({ id: 5, status: "ready", sort_index: 5 });
    const state = baseState([a, b, c]);
    expect(computeUpNext(state).map((i) => i.id)).toEqual([4, 5, 3]);
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

  test("Git history is discoverable in the launcher directory", () => {
    // The rail no longer has a curated "Pages" subset — the launcher is
    // the single discovery surface, so every page (incl. Git History)
    // appears here.
    const entries = computePagesDirectory({ backlogReadyCount: 0 });
    expect(entries.find((e) => e.id === "git-history")).toBeDefined();
  });

  test("every entry is tagged with a category, grouped in category order", () => {
    const entries = computePagesDirectory({ backlogReadyCount: 0 });
    expect(entries.every((e) => typeof e.category === "string" && e.category.length > 0)).toBe(true);
    // Entries are listed grouped by category so the flat launcher order
    // reads top-to-bottom by section; assert no category is interleaved.
    const seen = new Set<string>();
    let prev = "";
    for (const e of entries) {
      if (e.category !== prev) {
        expect(seen.has(e.category)).toBe(false);
        seen.add(e.category);
        prev = e.category;
      }
    }
  });

  test("Backlog badge surfaces only when backlogReadyCount is > 0", () => {
    expect(computePagesDirectory({ backlogReadyCount: 0 }).find((e) => e.id === "backlog")?.badge).toBeUndefined();
    expect(computePagesDirectory({ backlogReadyCount: 3 }).find((e) => e.id === "backlog")?.badge).toBe(3);
  });

  test("includes the four work pages in plan→done→backlog→archived order", () => {
    const entries = computePagesDirectory({ backlogReadyCount: 0 });
    const ids = entries.map((e) => e.id);
    expect(ids.indexOf("tasks")).toBeGreaterThanOrEqual(0);
    expect(ids.indexOf("tasks")).toBeLessThan(ids.indexOf("done-work"));
    expect(ids.indexOf("done-work")).toBeLessThan(ids.indexOf("backlog"));
    expect(ids.indexOf("backlog")).toBeLessThan(ids.indexOf("archived"));
  });
});
