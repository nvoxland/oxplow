import { describe, expect, test } from "bun:test";
import type { Extension, Lens } from "../tauri-bridge/generated/bindings.js";
import { slotRuns, mergeDirectory, barRows, childParams, lineSeries, numericRowId, treemapItems, cellLinkRef, changedParams, displayColumns, formatCell, lensDirectoryEntries, parseParamInput, shouldRerunLens, limitRows, slugify, adHocLens, rowMention, slotMounts, effortRowId } from "./lensModel.js";

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
  chart: null,
  children: [],
  launcherCategory: null,
  hidden: false,
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
        { key: "title", label: "Task", link: { kind: "task", from: "id", line: null, base: null, head: null } },
        { key: "missing", label: null, link: null },
        { key: "id", label: null, link: null },
      ],
    });
    expect(displayColumns(l, ["id", "title"])).toEqual([
      { key: "title", label: "Task", index: 1, link: { kind: "task", from: "id", line: null, base: null, head: null } },
      { key: "id", label: "id", index: 0, link: null },
    ]);
  });
});

describe("cellLinkRef", () => {
  const cols = ["id", "title", "path", "slug", "effort"];
  const row = [42, "Fix it", "src/a.ts", "auth-flow", 7];

  test("task link reads the id from `from`", () => {
    expect(cellLinkRef({ kind: "task", from: "id", line: null, base: null, head: null }, "title", row, cols)?.id).toBe("task:tsk42");
  });
  test("file / wiki / effort-diff links default to the column itself", () => {
    expect(cellLinkRef({ kind: "file", from: null, line: null, base: null, head: null }, "path", row, cols)?.id).toBe("file:src/a.ts");
    expect(cellLinkRef({ kind: "wiki", from: null, line: null, base: null, head: null }, "slug", row, cols)?.id).toBe("wiki:auth-flow");
    expect(cellLinkRef({ kind: "effort-diff", from: null, line: null, base: null, head: null }, "effort", row, cols)?.id).toBe("diff-view:effort:7");
  });
  test("null or missing target gives no link", () => {
    expect(cellLinkRef({ kind: "task", from: "id", line: null, base: null, head: null }, "title", [null, "x", null, null, null], cols)).toBeNull();
    expect(cellLinkRef({ kind: "task", from: "nope", line: null, base: null, head: null }, "title", row, cols)).toBeNull();
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
      { name: "review", description: "Review helpers", path: "oxplow/extensions/review", errors: [], lenses: [lens()], source: null, sources: [], origin: "project", slots: [], enabled: true },
      { name: "broken", description: "", path: "oxplow/extensions/broken", errors: ["bad"], lenses: [], source: null, sources: [], origin: "project", slots: [], enabled: true },
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

describe("rowMention", () => {
  test("a lens row as a one-line agent context mention", () => {
    expect(rowMention("review/waiting", ["id", "title", "note"], [42, "Fix it", null])).toBe(
      '[oxplow lens review/waiting row: id=42, title="Fix it", note=null] ',
    );
  });
});

describe("slotMounts", () => {
  test("lens ids mounted in a slot, in extension order", () => {
    const ext = (name: string, slots: { slot: string; lensId: string }[]): Extension => ({
      name, description: "", path: "", errors: [], lenses: [], source: null, sources: [], origin: "bundled", slots,
    });
    const exts = [
      ext("oxplow-review", [{ slot: "effort-review", lensId: "oxplow-review/decisions" }, { slot: "effort-review", lensId: "oxplow-review/claims" }]),
      ext("mine", [{ slot: "effort-review", lensId: "mine/x" }, { slot: "other", lensId: "mine/y" }]),
    ];
    expect(slotMounts(exts, "effort-review")).toEqual(["oxplow-review/decisions", "oxplow-review/claims", "mine/x"]);
  });
});

describe("effortRowId", () => {
  test("the numeric id v_* views use, from an effort id like eff262", () => {
    expect(effortRowId("eff262")).toBe(262);
    expect(effortRowId("262")).toBe(262);
    expect(effortRowId("nope")).toBeNull();
  });
});

describe("links added for extraction", () => {
  const cols = ["id", "path", "ln", "sha", "key"];
  const row = [42, "src/a.rs", 7, "abc123", "oxplow.complexity"];
  test("a task link from a v_task integer id opens tsk<id>", () => {
    expect(cellLinkRef({ kind: "task", from: "id", line: null, base: null, head: null }, "id", row, cols)?.id).toBe("task:tsk42");
    expect(cellLinkRef({ kind: "task", from: null, line: null, base: null, head: null }, "t", ["tsk9"], ["t"])?.id).toBe("task:tsk9");
  });
  test("file links can carry a line", () => {
    const ref = cellLinkRef({ kind: "file", from: "path", line: "ln", base: null, head: null }, "path", row, cols);
    expect(ref?.id).toBe("file:src/a.rs");
    expect((ref?.payload as { line?: number }).line).toBe(7);
  });
  test("commit and metric links", () => {
    expect(cellLinkRef({ kind: "commit", from: "sha", line: null, base: null, head: null }, "sha", row, cols)?.kind).toBe("git-commit");
    expect(cellLinkRef({ kind: "metric", from: "key", line: null, base: null, head: null }, "key", row, cols)?.id).toBe(
      "metric-detail:oxplow.complexity",
    );
  });
});

describe("launcher entries", () => {
  const ext = (over: Partial<Extension>): Extension =>
    ({
      name: "x",
      description: "",
      path: "",
      errors: [],
      lenses: [],
      source: null,
      sources: [],
      origin: "project",
      slots: [],
      enabled: true,
      ...over,
    }) as Extension;
  test("hidden lenses are skipped and categories are honored", () => {
    const entries = lensDirectoryEntries([
      ext({
        lenses: [
          lens({ id: "x/a", slug: "a", launcherCategory: "Activity" }),
          lens({ id: "x/b", slug: "b", hidden: true }),
          lens({ id: "x/c", slug: "c" }),
        ],
      }),
    ]);
    expect(entries.map((e) => [e.ref.id, e.category])).toEqual([
      ["lens:x/a", "Activity"],
      ["lens:x/c", "Lenses"],
    ]);
  });
});

describe("chart data", () => {
  const result = (columns: string[], rows: (string | number | null)[][]) => ({ columns, rows, truncated: false });
  test("bar rows read chart.x / chart.y", () => {
    const l = lens({ viz: "bar", chart: { x: "day", y: "n", series: null, label: null, size: null, group: null } });
    expect(barRows(l, result(["day", "n"], [["mon", 3], ["tue", null]]))).toEqual([
      { label: "mon", value: 3 },
      { label: "tue", value: 0 },
    ]);
  });
  test("line series split by chart.series and parse times", () => {
    const l = lens({ viz: "line", chart: { x: "at", y: "v", series: "s", label: null, size: null, group: null } });
    const out = lineSeries(
      l,
      result(["at", "v", "s"], [["2026-01-01T00:00:00Z", 1, "a"], ["2026-01-02T00:00:00Z", 2, "a"], [5, 9, "b"]]),
    );
    expect(out.map((s) => s.name)).toEqual(["a", "b"]);
    expect(out[0]!.points).toEqual([
      { t: Date.parse("2026-01-01T00:00:00Z"), v: 1 },
      { t: Date.parse("2026-01-02T00:00:00Z"), v: 2 },
    ]);
    expect(out[1]!.points).toEqual([{ t: 5, v: 9 }]);
  });
  test("treemap items drop non-positive sizes and keep their row", () => {
    const l = lens({ viz: "treemap", chart: { x: null, y: null, series: null, label: "p", size: "c", group: "z" } });
    const items = treemapItems(l, result(["p", "c", "z"], [["a.rs", 5, "core"], ["b.rs", 0, "ui"]]));
    expect(items).toEqual([{ label: "a.rs", size: 5, group: "core", row: ["a.rs", 5, "core"] }]);
  });
});

test("grid children get only the params they declare", () => {
  const child = lens({ params: [{ name: "effort_id", label: null, default: null }] });
  expect(childParams(child, { effort_id: 3, other: "x" })).toEqual({ effort_id: 3 });
});

test("numericRowId strips a UI id prefix", () => {
  expect(numericRowId("tsk42")).toBe(42);
  expect(numericRowId("thr7")).toBe(7);
  expect(numericRowId("eff262")).toBe(262);
  expect(numericRowId("12")).toBe(12);
  expect(numericRowId("abc")).toBeNull();
});

test("mergeDirectory slots lens entries into their categories, keeping category order", () => {
  const e = (id: string, category: string) => ({ id, label: id, ref: { id, kind: "lens", payload: null }, category }) as never;
  const merged = mergeDirectory(
    [e("tasks", "Work"), e("git", "Git"), e("settings", "System")],
    [e("usage", "Activity"), e("mine", "Lenses"), e("plan", "Work")],
  );
  expect(merged.map((m: { id: string }) => m.id)).toEqual(["tasks", "plan", "git", "usage", "mine", "settings"]);
});

describe("change links", () => {
  const cols = ["path", "ln", "base", "head", "dup"];
  test("diff-at opens the file's diff between the change's two sides, at the line", () => {
    const ref = cellLinkRef(
      { kind: "diff-at", from: "path", line: "ln", base: "base", head: "head" },
      "path",
      ["src/a.rs", 12, "abc1234", "def5678", null],
      cols,
    )!;
    expect(ref.kind).toBe("diff");
    const p = ref.payload as { path: string; leftVersion: unknown; rightVersion: unknown; revealLine?: number };
    expect(p.path).toBe("src/a.rs");
    expect(p.leftVersion).toEqual({ kind: "ref", ref: "abc1234" });
    expect(p.rightVersion).toEqual({ kind: "ref", ref: "def5678" });
    expect(p.revealLine).toBe(12);
    const working = cellLinkRef(
      { kind: "diff-at", from: "path", line: null, base: "base", head: "head" },
      "path",
      ["src/a.rs", null, "HEAD", "working tree", null],
      cols,
    )!;
    expect((working.payload as { rightVersion: unknown }).rightVersion).toEqual({ kind: "disk" });
    expect(
      cellLinkRef({ kind: "diff-at", from: "path", line: null, base: "base", head: "head" }, "path", ["a", null, "snapshot 3", "snapshot 4", null], cols),
    ).toBeNull();
  });
  test("compare opens both ranges side by side at the change's version", () => {
    const ref = cellLinkRef(
      { kind: "compare", from: "dup", line: null, base: null, head: "head" },
      "dup",
      ["x", null, null, "def5678", "src/b.rs:3-12|src/a.rs:40-49"],
      cols,
    )!;
    expect(ref.kind).toBe("duplicate-block");
    expect(ref.payload).toEqual({
      leftPath: "src/b.rs",
      leftStart: 3,
      leftEnd: 12,
      leftVersion: { kind: "ref", ref: "def5678" },
      rightPath: "src/a.rs",
      rightStart: 40,
      rightEnd: 49,
      rightVersion: { kind: "ref", ref: "def5678" },
    });
    expect(cellLinkRef({ kind: "compare", from: "dup", line: null, base: null, head: null }, "dup", ["x", null, null, null, "garbage"], cols)).toBeNull();
  });
});

test("slot lenses get only the slot params they declare", () => {
  const exts = [
    {
      name: "x",
      enabled: true,
      slots: [{ slot: "effort-review", lensId: "x/a" }],
      lenses: [lens({ id: "x/a", params: [{ name: "effort_id", label: null, default: null }] })],
    },
  ] as unknown as Extension[];
  expect(slotRuns(exts, "effort-review", { effort_id: 7, change_id: 9 })).toEqual([{ id: "x/a", params: { effort_id: 7 } }]);
});

test("page links open any oxplow page by its tab id", () => {
  expect(cellLinkRef({ kind: "page", from: null, line: null, base: null, head: null }, "p", ["task:tsk3"], ["p"])?.id).toBe("task:tsk3");
  expect(cellLinkRef({ kind: "page", from: null, line: null, base: null, head: null }, "p", ["git-dashboard"], ["p"])?.kind).toBe("git-dashboard");
});
