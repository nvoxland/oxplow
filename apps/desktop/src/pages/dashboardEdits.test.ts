import { describe, expect, test } from "bun:test";
import { parse } from "yaml";

import { addTile, moveTile, newDashboard, removeTile, rename } from "./dashboardEdits.js";

const source = `# A hand-authored dashboard
title: "Build health" # keep the title note
description: |
  A summary of the build.
category: Dashboards
params: [{ name: branch, default: main }]
view:
  grid:
    # Tests belong together
    - id: tests
      title: "Tests" # tile title note
      size: wide
      embed: { page: oxplow/problems }
    # Coverage follows
    - id: coverage
      embed:
        page: my-pages/coverage
        params: { branch: { param: branch } }
rail: { markdown: "A useful note" } # keep the rail
`;

const ids = (yaml: string): string[] => parse(yaml).view.grid.map((tile: { id: string }) => tile.id);

describe("dashboard document edits", () => {
  test("new dashboards are titled empty grid pages; blank titles are refused", () => {
    expect(parse(newDashboard("  Release: health  "))).toEqual({
      title: "Release: health",
      icon: "layout-dashboard",
      category: "Dashboards",
      view: { grid: [] },
    });
    expect(() => newDashboard(" \n ")).toThrow("title");
  });

  test("renaming keeps title comments, quoted style, key order and unrelated content", () => {
    const renamed = rename(source, "Release health");
    expect(renamed).toContain('title: "Release health" # keep the title note');
    expect(renamed).toContain("# A hand-authored dashboard");
    expect(renamed).toContain("description: |\n  A summary of the build.");
    expect(renamed).toContain("# keep the rail");
    expect(renamed.indexOf("description:")).toBeLessThan(renamed.indexOf("category:"));
    const before = parse(source);
    before.title = "Release health";
    expect(parse(renamed)).toEqual(before);
    expect(() => rename(source, " ")).toThrow("title");
  });

  test("adding an embed preserves old nodes and inserts at the requested index", () => {
    const added = addTile(source, {
      id: "releases",
      page: "acme/recent-releases",
      title: "Releases",
      size: "full",
      params: { branch: { param: "branch" }, count: 5 },
    }, 1);
    expect(ids(added)).toEqual(["tests", "releases", "coverage"]);
    expect(parse(added).view.grid[1]).toEqual({
      id: "releases", title: "Releases", size: "full",
      embed: { page: "acme/recent-releases", params: { branch: { param: "branch" }, count: 5 } },
    });
    expect(added).toContain("# Tests belong together");
    expect(added).toContain('title: "Tests" # tile title note');
    expect(added).toContain("# Coverage follows");
    expect(parse(added).view.grid[0]).toEqual(parse(source).view.grid[0]);
    expect(parse(added).view.grid[2]).toEqual(parse(source).view.grid[1]);
    expect(ids(addTile(source, { id: "last", page: "acme/last" }))).toEqual(["tests", "coverage", "last"]);
  });

  test("moving retains each tile's comments and uses its final index in both directions", () => {
    const moved = moveTile(source, "tests", 1);
    expect(ids(moved)).toEqual(["coverage", "tests"]);
    // YAML attaches a comment before the first item to the grid itself;
    // it stays with that collection, while node comments move with nodes.
    expect(moved).toContain("# Tests belong together");
    expect(moved.indexOf("# Coverage follows")).toBeLessThan(moved.indexOf("id: coverage"));
    expect(moved.indexOf("id: coverage")).toBeLessThan(moved.indexOf("# tile title note"));
    expect(moved).toContain('title: "Tests" # tile title note');
    expect(parse(moved).view.grid[1]).toEqual(parse(source).view.grid[0]);
    expect(ids(moveTile(moved, "tests", 0))).toEqual(["tests", "coverage"]);
    expect(moveTile(source, "tests", 0)).toBe(source);
  });

  test("removing retains the other tile and unrelated document comments", () => {
    const removed = removeTile(source, "tests");
    expect(ids(removed)).toEqual(["coverage"]);
    expect(removed).toContain("# Coverage follows");
    expect(removed).toContain("# keep the rail");
    expect(parse(removed).view.grid[0]).toEqual(parse(source).view.grid[1]);
    expect(parse(removeTile(removed, "coverage")).view.grid).toEqual([]);
  });

  test("tile edits refuse malformed documents, wrong layouts and ambiguous ids", () => {
    for (const invalid of [
      "title: [broken",
      "[]",
      "title: Other\nview: { grid: [] }",
      "category: Dashboards\nview: { stack: [] }",
      "category: Dashboards\nview: { grid: [], stack: [] }",
      "category: Dashboards\nview: { grid: {} }",
      "category: Dashboards\nview: { grid: [null] }",
      "category: Dashboards\nview: { grid: [{ embed: { page: acme/a } }] }",
      "category: Dashboards\nview: { grid: [{ id: '' }] }",
      "category: Dashboards\nview: { grid: [{ id: same }, { id: same }] }",
      "category: Dashboards\nview: { grid: [] }\nview: { grid: [] }",
    ]) {
      expect(() => addTile(invalid, { id: "new", page: "acme/new" })).toThrow();
      expect(() => removeTile(invalid, "tests")).toThrow();
      expect(() => moveTile(invalid, "tests", 0)).toThrow();
    }
  });

  test("bad tile references, duplicate additions and out-of-range positions are refused", () => {
    expect(() => removeTile(source, "missing")).toThrow("No dashboard tile");
    expect(() => moveTile(source, "missing", 0)).toThrow("No dashboard tile");
    expect(() => addTile(source, { id: "tests", page: "acme/new" })).toThrow("already");
    expect(() => addTile(source, { id: "", page: "acme/new" })).toThrow("id");
    expect(() => addTile(source, { id: "new", page: " " })).toThrow("page");
    for (const at of [-1, 0.5, 3, NaN, Infinity]) {
      expect(() => addTile(source, { id: "new", page: "acme/new" }, at)).toThrow("position");
    }
    for (const at of [-1, 0.5, 2, NaN, Infinity]) {
      expect(() => moveTile(source, "tests", at)).toThrow("position");
    }
  });
});
