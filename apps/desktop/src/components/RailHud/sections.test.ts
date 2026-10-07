import { describe, expect, test } from "bun:test";
import { computePagesDirectory } from "./sections.js";
import { gitDashboardRef, uncommittedChangesRef } from "../../tabs/pageRefs.js";

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

  test("includes the work pages in plan→done→backlog order", () => {
    const entries = computePagesDirectory({ backlogReadyCount: 0 });
    const ids = entries.map((e) => e.id);
    expect(ids.indexOf("tasks")).toBeGreaterThanOrEqual(0);
    expect(ids.indexOf("tasks")).toBeLessThan(ids.indexOf("done-work"));
    expect(ids.indexOf("done-work")).toBeLessThan(ids.indexOf("backlog"));
    expect(ids).not.toContain("archived");
  });
});
