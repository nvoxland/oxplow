import { describe, expect, test } from "bun:test";
import type { Extension, Lens } from "../tauri-bridge/generated/bindings.js";
import { cellLinkRef, changedParams, displayColumns, formatCell, lensDirectoryEntries, parseParamInput, shouldRerunLens, limitRows, slugify, adHocLens } from "./lensModel.js";

const lens = (over: Partial<Lens> = {}): Lens => ({
  id: "review/waiting",
  extension: "review",
  slug: "waiting",
  title: "Waiting on me",
  description: "Blocked tasks",
  query: "SELECT id, title FROM v_task",
  viz: "table",
  params: [],
  columns: [],
  empty: null,
  path: "oxplow/extensions/review/lenses/waiting.yaml",
  ...over,
});

describe("displayColumns", () => {
  test("without declared columns shows every result column by name", () => {
    expect(displayColumns(lens(), ["id", "title"])).toEqual([
      { key: "id", label: "id", index: 0, link: null },
      { key: "title", label: "title", index: 1, link: null },
    ]);
  });

  test("declared columns set order, labels and links, and skip keys the result lacks", () => {
    const l = lens({
      columns: [
        { key: "title", label: "Task", link: { kind: "task", from: "id" } },
        { key: "missing", label: null, link: null },
        { key: "id", label: null, link: null },
      ],
    });
    expect(displayColumns(l, ["id", "title"])).toEqual([
      { key: "title", label: "Task", index: 1, link: { kind: "task", from: "id" } },
      { key: "id", label: "id", index: 0, link: null },
    ]);
  });
});

describe("cellLinkRef", () => {
  const cols = ["id", "title", "path", "slug", "effort"];
  const row = [42, "Fix it", "src/a.ts", "auth-flow", 7];

  test("task link reads the id from `from`", () => {
    expect(cellLinkRef({ kind: "task", from: "id" }, "title", row, cols)?.id).toBe("task:42");
  });
  test("file / wiki / effort-diff links default to the column itself", () => {
    expect(cellLinkRef({ kind: "file", from: null }, "path", row, cols)?.id).toBe("file:src/a.ts");
    expect(cellLinkRef({ kind: "wiki", from: null }, "slug", row, cols)?.id).toBe("wiki:auth-flow");
    expect(cellLinkRef({ kind: "effort-diff", from: null }, "effort", row, cols)?.id).toBe("diff-view:effort:7");
  });
  test("null or missing target gives no link", () => {
    expect(cellLinkRef({ kind: "task", from: "id" }, "title", [null, "x", null, null, null], cols)).toBeNull();
    expect(cellLinkRef({ kind: "task", from: "nope" }, "title", row, cols)).toBeNull();
  });
});

describe("formatCell", () => {
  test("renders scalars as text", () => {
    expect(formatCell(null)).toBe("—");
    expect(formatCell(1.5)).toBe("1.5");
    expect(formatCell(1200)).toBe("1200"); // ids must not get grouping separators
    expect(formatCell(true)).toBe("yes");
    expect(formatCell("x")).toBe("x");
  });
});

describe("lensDirectoryEntries", () => {
  test("one launcher entry per loaded lens, under Lenses", () => {
    const exts: Extension[] = [
      { name: "review", description: "Review helpers", path: "oxplow/extensions/review", errors: [], lenses: [lens()] },
      { name: "broken", description: "", path: "oxplow/extensions/broken", errors: ["bad"], lenses: [] },
    ];
    const entries = lensDirectoryEntries(exts);
    expect(entries).toHaveLength(1);
    expect(entries[0]!.id).toBe("lens:review/waiting");
    expect(entries[0]!.label).toBe("Waiting on me");
    expect(entries[0]!.category).toBe("Lenses");
    expect(entries[0]!.ref.kind).toBe("lens");
    expect(entries[0]!.keywords).toContain("review");
    expect(entries[0]!.keywords).toContain("Blocked tasks");
  });
});

describe("parseParamInput", () => {
  test("numbers stay numbers, everything else is text, blank is null", () => {
    expect(parseParamInput("3")).toBe(3);
    expect(parseParamInput("2.5")).toBe(2.5);
    expect(parseParamInput("done")).toBe("done");
    expect(parseParamInput("  ")).toBeNull();
    expect(parseParamInput("007x")).toBe("007x");
  });
});

describe("changedParams", () => {
  test("keeps only values that differ from the lens defaults", () => {
    const l = lens({ params: [{ name: "a", label: null, default: 1 }, { name: "b", label: null, default: "x" }] });
    expect(changedParams(l, { a: 1, b: "y" })).toEqual({ b: "y" });
    expect(changedParams(l, {})).toEqual({});
  });
});

describe("shouldRerunLens", () => {
  test("data events re-run; UI bookkeeping doesn't; file edits only under oxplow/extensions", () => {
    expect(shouldRerunLens({ kind: "tasksChanged" })).toBe(true);
    expect(shouldRerunLens({ kind: "pageVisitChanged" })).toBe(false);
    expect(shouldRerunLens({ kind: "usageRecorded" })).toBe(false);
    expect(shouldRerunLens({ kind: "workspaceChanged", path: "src/main.rs" })).toBe(false);
    expect(shouldRerunLens({ kind: "workspaceChanged", path: "oxplow/extensions/review/lenses/a.yaml" })).toBe(true);
  });
});

describe("limitRows", () => {
  test("caps rows for compact views and marks the result truncated", () => {
    const r = { columns: ["n"], rows: [[1], [2], [3]], truncated: false };
    expect(limitRows(r, 2)).toEqual({ columns: ["n"], rows: [[1], [2]], truncated: true });
    expect(limitRows(r, 5)).toBe(r);
    expect(limitRows(r, undefined)).toBe(r);
  });
});

describe("slugify", () => {
  test("titles become lowercase-dash slugs valid for lens files", () => {
    expect(slugify("Open Tasks by Thread")).toBe("open-tasks-by-thread");
    expect(slugify("  What's  waiting?? ")).toBe("what-s-waiting");
    expect(slugify("---")).toBe("lens");
  });
});

describe("adHocLens", () => {
  test("wraps an Explore Data query as an unsaved lens for rendering", () => {
    const l = adHocLens("SELECT 1", "number");
    expect(l.query).toBe("SELECT 1");
    expect(l.viz).toBe("number");
    expect(l.columns).toEqual([]);
    expect(l.id).toBe("explore/ad-hoc");
  });
});
