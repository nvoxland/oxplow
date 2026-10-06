import { describe, expect, test } from "bun:test";
import type { Extension, Lens } from "../tauri-bridge/generated/bindings.js";
import { treeNodes, timelineEntries, stepItems, hunkRows, slotRuns, mergeDirectory, barRows, childParams, lineSeries, numericRowId, treemapItems, cellLinkRef, changedParams, displayColumns, formatCell, lensDirectoryEntries, parseParamInput, limitRows, slugify, adHocLens, rowMention, rowAsk, effortRowId, slotExtensions, isTimestamp, rowGroups, rowEmphasized, rowDepth, rowContextRef } from "./lensModel.js";
import { formatMetricValue, formatShortDateTime } from "../components/format.js";

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
  tree: null,
  timeline: null,
  steps: null,
  hunks: null,
  form: null,
  children: [],
  launcherCategory: null,
  hidden: false,
  path: "oxplow/extensions/review/lenses/waiting.yaml",
  ...over,
});

describe("displayColumns", () => {
  test("without declared columns shows every result column by name", () => {
    expect(displayColumns(lens(), ["id", "title"])).toEqual([
      { key: "id", label: "id", index: 0, link: null, unitIndex: null, iconIndex: null, toneIndex: null },
      { key: "title", label: "title", index: 1, link: null, unitIndex: null, iconIndex: null, toneIndex: null },
    ]);
  });

  test("declared columns set order, labels, links and units, and skip keys the result lacks", () => {
    const l = lens({
      columns: [
        { key: "title", label: "Task", link: { kind: "task", from: "id", line: null, base: null, head: null }, unit: null },
        { key: "missing", label: null, link: null, unit: null },
        { key: "id", label: null, link: null, unit: "u" },
      ],
    });
    expect(displayColumns(l, ["id", "title", "u"])).toEqual([
      { key: "title", label: "Task", index: 1, link: { kind: "task", from: "id", line: null, base: null, head: null }, unitIndex: null, iconIndex: null, toneIndex: null },
      { key: "id", label: "id", index: 0, link: null, unitIndex: 2, iconIndex: null, toneIndex: null },
    ]);
  });

  // tsk1089: a column's icon and tone come from other columns of the row;
  // the row-styling columns (group, emphasis, depth) are never cells.
  test("columns carry their icon and tone columns; styling columns aren't cells", () => {
    const styled = lens({
      group: { by: "bucket", link: null },
      emphasis: "on",
      depth: "d",
      columns: [{ key: "title", label: null, link: null, unit: null, icon: "glyph", tone: "hue" }, { key: "bucket", label: null, link: null, unit: null }],
    });
    expect(displayColumns(styled, ["title", "bucket", "glyph", "hue"])).toEqual([
      { key: "title", label: "title", index: 0, link: null, unitIndex: null, iconIndex: 2, toneIndex: 3 },
    ]);
    const auto = lens({ group: { by: "bucket", link: null }, emphasis: "on", depth: "d" });
    expect(displayColumns(auto, ["title", "bucket", "on", "d"]).map((c) => c.key)).toEqual(["title"]);
  });
});

describe("row groups and styling (tsk1089)", () => {
  const cols = ["bucket", "title", "ref", "on", "d"];
  const rows = [
    ["Ready", "a", "page:tasks", 0, 0],
    ["Done", "b", null, 1, 1],
    ["Ready", "c", "page:other", 0, "2"],
  ];
  const res = { columns: cols, rows, truncated: false } as never;

  test("rows group by a column, in the order each value first appears; the heading links from the group's first row", () => {
    const l = lens({ viz: "list", group: { by: "bucket", link: { kind: "page", from: "ref", line: null, base: null, head: null } } });
    const groups = rowGroups(l, res)!;
    expect(groups.map((g) => [g.key, g.label, g.rows.map((r) => r[1])])).toEqual([
      ["Ready", "Ready", ["a", "c"]],
      ["Done", "Done", ["b"]],
    ]);
    expect(groups[0]!.ref?.id).toBe("page:tasks");
    expect(groups[1]!.ref).toBeNull();
  });

  test("an ungrouped lens, or one whose group column isn't in the result, has no groups", () => {
    expect(rowGroups(lens({ viz: "list" }), res)).toBeNull();
    expect(rowGroups(lens({ viz: "list", group: { by: "gone", link: null } }), res)).toBeNull();
  });

  test("emphasis is a truthy cell; depth an integer cell, else 0", () => {
    const l = lens({ emphasis: "on", depth: "d" });
    expect(rows.map((r) => rowEmphasized(l, cols, r))).toEqual([false, true, false]);
    expect(rows.map((r) => rowDepth(l, cols, r))).toEqual([0, 1, 2]);
    expect(rowEmphasized(lens(), cols, rows[1]!)).toBe(false);
    expect(rowDepth(lens({ depth: "title" }), cols, rows[0]!)).toBe(0);
    expect(rowEmphasized(lens({ emphasis: "title" }), cols, ["x", "false"])).toBe(false);
  });

  test("a row that links somewhere drags into the agent's context", () => {
    const link = (kind: "task" | "wiki" | "file" | "page", from: string) => ({ kind, from, line: null, base: null, head: null });
    const at = (kind: "task" | "wiki" | "file" | "page", value: string | number) =>
      rowContextRef(lens({ columns: [{ key: "title", label: null, link: link(kind, "v"), unit: null }] }), ["title", "v"], ["T", value]);
    expect(at("task", 12)).toEqual({ kind: "ref", ref: "work_item:oxplow:tsk12" });
    expect(at("page", "page:comments")).toEqual({ kind: "ref", ref: "page:comments" });
    expect(at("wiki", "auth-flow")).toEqual({ kind: "wiki", slug: "auth-flow" });
    expect(at("file", "src/a.ts")).toEqual({ kind: "file", path: "src/a.ts" });
    expect(rowContextRef(lens(), ["title"], ["T"])).toBeNull();
  });
});

describe("cellLinkRef", () => {
  const cols = ["id", "title", "path", "slug", "effort"];
  const row = [42, "Fix it", "src/a.ts", "auth-flow", 7];

  test("task link reads the id from `from`", () => {
    expect(cellLinkRef({ kind: "task", from: "id", line: null, base: null, head: null }, "title", row, cols)?.id).toBe("work_item:oxplow:tsk42");
  });
  test("file / wiki / effort-diff links default to the column itself", () => {
    expect(cellLinkRef({ kind: "file", from: null, line: null, base: null, head: null }, "path", row, cols)?.id).toBe("file:src/a.ts");
    expect(cellLinkRef({ kind: "wiki", from: null, line: null, base: null, head: null }, "slug", row, cols)?.id).toBe("wiki:auth-flow");
    expect(cellLinkRef({ kind: "effort-diff", from: null, line: null, base: null, head: null }, "effort", row, cols)?.id).toBe("effort:eff7");
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
  // tsk1038: one time format — a stored timestamp shows in local time, as
  // the rest of the app does; a day (a chart bucket) stays a day.
  test("a timestamp shows in local time; a date stays a date", () => {
    const at = "2026-10-05T04:55:02.914532Z";
    expect(formatCell(at)).toBe(formatShortDateTime(at));
    expect(formatCell(at)).not.toContain("T04:55");
    expect(formatCell("2026-10-05")).toBe("2026-10-05");
    expect(isTimestamp(at)).toBe(true);
    expect(isTimestamp("2026-10-05")).toBe(false);
  });
  test("a number with a unit shows it", () => {
    expect(formatCell(16446, "ms")).toBe(formatMetricValue(16446, "ms"));
    expect(formatCell(42.5, "%")).toBe("42.5%");
    expect(formatCell(7, null)).toBe("7");
  });
});

describe("lensDirectoryEntries", () => {
  test("one launcher entry per loaded lens, under Lenses", () => {
    const exts: Extension[] = [
      { name: "review", description: "Review helpers", path: "oxplow/extensions/review", errors: [], lenses: [lens()], source: null, collectors: [], origin: "project", ui: { slots: [], commands: [], decorators: [] }, enabled: true },
      { name: "broken", description: "", path: "oxplow/extensions/broken", errors: ["bad"], lenses: [], source: null, collectors: [], origin: "project", ui: { slots: [], commands: [], decorators: [] }, enabled: true },
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

describe("rowAsk", () => {
  test("Ask About This on a row asks about what it links to, else the row itself", () => {
    const linked = lens({ columns: [{ key: "sha", label: null, link: { kind: "commit", from: null, line: null } }] });
    expect(rowAsk(linked, ["sha", "subject"], ["abc123", "Fix"])).toBe("[oxplow ref commit:abc123] ");
    expect(rowAsk(lens(), ["id", "title"], [42, "Fix it"])).toBe('[oxplow lens review/waiting row: id=42, title="Fix it"] ');
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
    expect(cellLinkRef({ kind: "task", from: "id", line: null, base: null, head: null }, "id", row, cols)?.id).toBe("work_item:oxplow:tsk42");
    expect(cellLinkRef({ kind: "task", from: null, line: null, base: null, head: null }, "t", ["tsk9"], ["t"])?.id).toBe("work_item:oxplow:tsk9");
  });
  test("file links can carry a line", () => {
    const ref = cellLinkRef({ kind: "file", from: "path", line: "ln", base: null, head: null }, "path", row, cols);
    expect(ref?.id).toBe("file:src/a.rs");
    expect((ref?.payload as { line?: number }).line).toBe(7);
  });
  test("commit and metric links", () => {
    expect(cellLinkRef({ kind: "commit", from: "sha", line: null, base: null, head: null }, "sha", row, cols)?.kind).toBe("commit");
    expect(cellLinkRef({ kind: "metric", from: "key", line: null, base: null, head: null }, "key", row, cols)?.id).toBe(
      "metric:oxplow.complexity",
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
      collectors: [],
      origin: "project",
      ui: { slots: [], commands: [], decorators: [] },
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
      ["src/a.rs", 12, "git:abc1234", "git:def5678", null],
      cols,
    )!;
    expect(ref.kind).toBe("diff");
    const p = ref.payload as { path: string; leftVersion: unknown; rightVersion: unknown; revealLine?: number };
    expect(p.path).toBe("src/a.rs");
    expect(p.leftVersion).toEqual("git:abc1234");
    expect(p.rightVersion).toEqual("git:def5678");
    expect(p.revealLine).toBe(12);
    const working = cellLinkRef(
      { kind: "diff-at", from: "path", line: null, base: "base", head: "head" },
      "path",
      ["src/a.rs", null, "git:HEAD", "working", null],
      cols,
    )!;
    expect((working.payload as { rightVersion: unknown }).rightVersion).toEqual("working");
    // An effort's change is between two snapshots; those open too.
    const effort = cellLinkRef(
      { kind: "diff-at", from: "path", line: null, base: "base", head: "head" },
      "path",
      ["a", null, "snap:3", "snap:4", null],
      cols,
    )!;
    expect((effort.payload as { leftVersion: unknown }).leftVersion).toEqual("snap:3");
    expect((effort.payload as { rightVersion: unknown }).rightVersion).toEqual("snap:4");
    // A cell that isn't a revision opens nothing rather than a guess.
    expect(
      cellLinkRef(
        { kind: "diff-at", from: "path", line: null, base: "base", head: "head" },
        "path",
        ["a", null, "abc1234", "working tree", null],
        cols,
      ),
    ).toBeNull();
  });
  test("compare opens both ranges side by side at the change's version", () => {
    const ref = cellLinkRef(
      { kind: "compare", from: "dup", line: null, base: null, head: "head" },
      "dup",
      ["x", null, null, "git:def5678", "src/b.rs:3-12|src/a.rs:40-49"],
      cols,
    )!;
    expect(ref.kind).toBe("duplicate-block");
    expect(ref.payload).toEqual({
      leftPath: "src/b.rs",
      leftStart: 3,
      leftEnd: 12,
      leftVersion: "git:def5678",
      rightPath: "src/a.rs",
      rightStart: 40,
      rightEnd: 49,
      rightVersion: "git:def5678",
    });
    expect(cellLinkRef({ kind: "compare", from: "dup", line: null, base: null, head: null }, "dup", ["x", null, null, null, "garbage"], cols)).toBeNull();
  });
});

test("slot lenses get only the slot params they declare", () => {
  const exts = [
    {
      name: "x",
      enabled: true,
      ui: { slots: [{ slot: "effort.review.details", lensId: "x/a" }] },
      lenses: [lens({ id: "x/a", params: [{ name: "effort_id", label: null, default: null }] })],
    },
  ] as unknown as Extension[];
  expect(slotRuns(exts, "effort.review.details", { effort_id: 7, change_id: 9 })).toEqual([{ id: "x/a", params: { effort_id: 7 } }]);
});

test("a slot can be narrowed to one extension, and lists who mounts there", () => {
  const exts = [
    { name: "a", enabled: true, ui: { slots: [{ slot: "settings.section", lensId: "a/x" }] }, lenses: [lens({ id: "a/x" })] },
    { name: "b", enabled: true, ui: { slots: [{ slot: "settings.section", lensId: "b/y" }] }, lenses: [lens({ id: "b/y" })] },
    { name: "c", enabled: false, ui: { slots: [{ slot: "settings.section", lensId: "c/z" }] }, lenses: [lens({ id: "c/z" })] },
    { name: "d", enabled: true, ui: { slots: [{ slot: "thread.plan.header", lensId: "d/w" }] }, lenses: [lens({ id: "d/w" })] },
  ] as unknown as Extension[];
  expect(slotRuns(exts, "settings.section", {}, "b")).toEqual([{ id: "b/y", params: {} }]);
  expect(slotExtensions(exts, "settings.section")).toEqual(["a", "b"]);
});

test("page links open any oxplow page by its tab id", () => {
  expect(cellLinkRef({ kind: "page", from: null, line: null, base: null, head: null }, "p", ["work_item:oxplow:tsk3"], ["p"])?.id).toBe("work_item:oxplow:tsk3");
  expect(cellLinkRef({ kind: "page", from: null, line: null, base: null, head: null }, "p", ["page:git-dashboard"], ["p"])?.kind).toBe("git-dashboard");
});

const result = (columns: string[], rows: (string | number | null)[][]) => ({
  columns,
  rows,
  truncated: false,
  reads: { models: [], tables: [], measures: [] },
  freshness: [],
});

describe("structure components (P6.A2)", () => {
  test("treeNodes nests rows under their parents; orphans are roots", () => {
    const l = lens({ viz: "tree", tree: { id: "id", parent: "p", label: "n" } });
    const nodes = treeNodes(
      l,
      result(["id", "p", "n"], [
        ["b", "a", "child"],
        ["a", null, "root"],
        ["c", "b", "grandchild"],
        ["d", "gone", "orphan"],
      ]),
    );
    const shape = (n: { label: string; children: unknown[] }): unknown => ({
      label: n.label,
      children: (n.children as { label: string; children: unknown[] }[]).map(shape),
    });
    expect(nodes.map(shape)).toEqual([
      { label: "root", children: [{ label: "child", children: [{ label: "grandchild", children: [] }] }] },
      { label: "orphan", children: [] },
    ]);
  });

  test("treeNodes survives a cycle", () => {
    const l = lens({ viz: "tree", tree: { id: "id", parent: "p", label: "id" } });
    expect(treeNodes(l, result(["id", "p"], [["a", "b"], ["b", "a"]]))).toEqual([]);
  });

  test("timelineEntries sort oldest first and carry refs", () => {
    const l = lens({ viz: "timeline", timeline: { at: "at", label: "w", ref: "r" } });
    const out = timelineEntries(
      l,
      result(["at", "w", "r"], [
        ["2026-09-30", "shipped", "commit:abc"],
        ["2026-09-29", "started", null],
      ]),
    );
    expect(out.map((e) => [e.at, e.label, e.ref])).toEqual([
      ["2026-09-29", "started", null],
      ["2026-09-30", "shipped", "commit:abc"],
    ]);
  });

  test("stepItems map statuses, anything unknown pending", () => {
    const l = lens({ viz: "steps", steps: { label: "s", status: "st" } });
    expect(stepItems(l, result(["s", "st"], [["plan", "done"], ["build", "active"], ["test", "failed"], ["ship", "later"]]))).toEqual([
      { label: "plan", status: "done", row: ["plan", "done"] },
      { label: "build", status: "active", row: ["build", "active"] },
      { label: "test", status: "failed", row: ["test", "failed"] },
      { label: "ship", status: "pending", row: ["ship", "later"] },
    ]);
  });

  test("hunkRows keep rows naming a file and two revisions", () => {
    const l = lens({ viz: "hunks", hunks: { path: "p", from: "a", to: "b" } });
    expect(
      hunkRows(l, result(["p", "a", "b"], [["x.rs", "git:abc", "working"], ["y.rs", null, "working"]])),
    ).toEqual([{ path: "x.rs", from: "git:abc", to: "working", row: ["x.rs", "git:abc", "working"] }]);
  });
});

describe("adHocLens charts", () => {
  test("an ad-hoc chart carries its chart columns", () => {
    const chart = { x: "day", y: "n", series: null, label: null, size: null, group: null };
    expect(adHocLens("SELECT 1", "bar", chart).chart).toEqual(chart);
    expect(adHocLens("SELECT 1", "table").chart).toBeNull();
  });
});
