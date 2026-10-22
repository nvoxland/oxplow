import { describe, expect, test } from "bun:test";
import type { Extension, Lens, LensRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { markParts, treeNodes, timelineEntries, stepItems, hunkRows, codeRows, diffRow, parseLines, slotRuns, mergeDirectory, barRows, childParams, partParams, lineSeries, sparklineValues, numericRowId, treemapItems, cellLinkRef, changedParams, lensViewChange, misplacedViewParams, foldEmpty, displayColumns, formatCell, lensDirectoryEntries, parseParamInput, limitRows, slugify, adHocLens, rowMention, rowAsk, effortRowId, slotExtensions, isTimestamp, rowGroups, rowEmphasized, rowDepth, rowContextRef } from "./lensModel.js";
import { formatMetricValue, formatShortDateTime } from "../components/format.js";
import { diffSpecOf, duplicateBlockOf, isPage, lineOf, pagePayload } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";

const lens = (over: Partial<Lens> = {}): Lens => ({
  id: "review/waiting",
  extension: "review",
  slug: "waiting",
  title: "Waiting on me",
  description: "Blocked tasks",
  query: "SELECT ref, title FROM v_work_item",
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
  parts: [],
  launcherCategory: null,
  hidden: false,
  path: ".oxplow/extensions/review/lenses/waiting.yaml",
  ...over,
});

describe("displayColumns", () => {
  test("without declared columns shows every result column, its key in words", () => {
    expect(displayColumns(lens(), ["uri", "title"])).toEqual([
      { key: "uri", label: "Uri", index: 0, link: null, unitIndex: null, iconIndex: null, toneIndex: null, width: null, format: null, highlight: null },
      { key: "title", label: "Title", index: 1, link: null, unitIndex: null, iconIndex: null, toneIndex: null, width: null, format: null, highlight: null },
    ]);
  });

  test("declared columns set order, labels, links and units, and skip keys the result lacks", () => {
    const l = lens({
      columns: [
        { key: "title", label: "Task", link: { kind: "page", from: "id", line: null, base: null, head: null }, unit: null },
        { key: "missing", label: null, link: null, unit: null },
        { key: "id", label: null, link: null, unit: "u" },
      ],
    });
    expect(displayColumns(l, ["id", "title", "u"])).toEqual([
      { key: "title", label: "Task", index: 1, link: { kind: "page", from: "id", line: null, base: null, head: null }, unitIndex: null, iconIndex: null, toneIndex: null, width: null, format: null, highlight: null },
      { key: "id", label: "Id", index: 0, link: null, unitIndex: 2, iconIndex: null, toneIndex: null, width: null, format: null, highlight: null },
    ]);
  });

  // A column's icon and tone come from other columns of the row;
  // the row-styling columns (group, emphasis, depth) are never cells.
  test("columns carry their icon and tone columns; styling columns aren't cells", () => {
    const styled = lens({
      group: { by: "bucket", link: null },
      emphasis: "on",
      depth: "d",
      columns: [{ key: "title", label: null, link: null, unit: null, icon: "glyph", tone: "hue" }, { key: "bucket", label: null, link: null, unit: null }],
    });
    expect(displayColumns(styled, ["title", "bucket", "glyph", "hue"])).toEqual([
      { key: "title", label: "Title", index: 0, link: null, unitIndex: null, iconIndex: 2, toneIndex: 3, width: null, format: null, highlight: null },
    ]);
    const auto = lens({ group: { by: "bucket", link: null }, emphasis: "on", depth: "d" });
    expect(displayColumns(auto, ["title", "bucket", "on", "d"]).map((c) => c.key)).toEqual(["title"]);
  });
});

describe("row groups and styling", () => {
  const cols = ["bucket", "title", "uri", "on", "d"];
  const rows = [
    ["Ready", "a", "oxplow:oxplow/page?page=tasks", 0, 0],
    ["Done", "b", null, 1, 1],
    ["Ready", "c", "oxplow:oxplow/page?page=other", 0, "2"],
  ];
  const res = { columns: cols, rows, truncated: false } as never;

  test("rows group by a column, in the order each value first appears; the heading links from the group's first row", () => {
    const l = lens({ viz: "list", group: { by: "bucket", link: { kind: "page", from: "uri", line: null, base: null, head: null } } });
    const groups = rowGroups(l, res)!;
    expect(groups.map((g) => [g.key, g.label, g.rows.map((r) => r[1])])).toEqual([
      ["Ready", "Ready", ["a", "c"]],
      ["Done", "Done", ["b"]],
    ]);
    expect(groups[0]!.ref).toBe("oxplow:oxplow/page?page=tasks");
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
    const link = (kind: "wiki" | "file" | "page", from: string) => ({ kind, from, line: null, base: null, head: null });
    const at = (kind: "wiki" | "file" | "page", value: string | number) =>
      rowContextRef(lens({ columns: [{ key: "title", label: null, link: link(kind, "v"), unit: null }] }), ["title", "v"], ["T", value]);
    expect(at("page", "oxplow:oxplow/page?page=comments")).toEqual({ kind: "ref", ref: "oxplow:oxplow/page?page=comments" });
    expect(at("wiki", "auth-flow")).toEqual({ kind: "wiki", slug: "auth-flow" });
    expect(at("file", "src/a.ts")).toEqual({ kind: "file", path: "src/a.ts" });
    expect(rowContextRef(lens(), ["title"], ["T"])).toBeNull();
  });
});

describe("cellLinkRef", () => {
  const cols = ["id", "title", "path", "slug", "effort"];
  const row = [42, "Fix it", "src/a.ts", "auth-flow", "oxplow:oxplow/effort?id=eff7"];

  test("a page link reads its ref from `from`", () => {
    expect(cellLinkRef({ kind: "page", from: "uri", line: null, base: null, head: null }, "title", [...row, "oxplow:oxplow/work_item?id=E-1&provider=issues"], [...cols, "uri"])).toBe("oxplow:oxplow/work_item?id=E-1&provider=issues");
  });
  test("file / wiki / effort-diff links default to the column itself", () => {
    expect(cellLinkRef({ kind: "file", from: null, line: null, base: null, head: null }, "path", row, cols)?.id).toBe("oxplow:oxplow/file?path=src/a.ts");
    expect(cellLinkRef({ kind: "wiki", from: null, line: null, base: null, head: null }, "slug", row, cols)?.id).toBe("oxplow:oxplow/wiki?slug=auth-flow");
    expect(cellLinkRef({ kind: "effort-diff", from: null, line: null, base: null, head: null }, "effort", row, cols)?.id).toBe("oxplow:oxplow/effort?id=eff7");
  });
  test("null or missing target gives no link", () => {
    expect(cellLinkRef({ kind: "file", from: "path", line: null, base: null, head: null }, "title", [null, "x", null, null, null], cols)).toBeNull();
    expect(cellLinkRef({ kind: "file", from: "nope", line: null, base: null, head: null }, "title", row, cols)).toBeNull();
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
      { name: "review", description: "Review helpers", path: ".oxplow/extensions/review", errors: [], lenses: [lens()], deployment: { kind: "deployed", hash: "0123456789abcdef" }, collectors: [], origin: "project", ui: { slots: [], commands: [], decorators: [] }, enabled: true },
      { name: "broken", description: "", path: ".oxplow/extensions/broken", errors: ["bad"], lenses: [], deployment: { kind: "deployed", hash: "0123456789abcdef" }, collectors: [], origin: "project", ui: { slots: [], commands: [], decorators: [] }, enabled: true },
    ];
    const entries = lensDirectoryEntries(exts);
    expect(entries).toHaveLength(1);
    expect(entries[0]!.id).toBe("oxplow:oxplow/lens?lens=review/waiting");
    expect(entries[0]!.label).toBe("Waiting on me");
    expect(entries[0]!.category).toBe("Lenses");
    expect(entries[0]!.ref.id).toBe("oxplow:oxplow/lens?lens=review/waiting");
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

// A lens page's params, set by its rail, a row's pick or its component,
// apart as its URI holds them: its resource params name what it shows, its
// filters and picks are its view. A filter replaces the entry; a pick, or
// another resource, pushes one.
describe("lensViewChange", () => {
  const l = lens({
    params: [
      { name: "days", label: null, default: 7, role: "filter" },
      { name: "task", label: null, default: null, role: "selection" },
      { name: "stream_id", label: null, default: null, role: "resource" },
    ],
  } as never);
  test("a filter replaces; values at their defaults leave the URI", () => {
    expect(lensViewChange(l, { task: "tsk1" }, { days: 14 })).toEqual({ resource: {}, view: { days: 14, task: "tsk1" }, push: false });
    expect(lensViewChange(l, { days: 14 }, { days: 7 })).toEqual({ resource: {}, view: {}, push: false });
  });
  test("a pick pushes; a cleared one replaces", () => {
    expect(lensViewChange(l, {}, { task: "tsk2" })).toEqual({ resource: {}, view: { task: "tsk2" }, push: true });
    expect(lensViewChange(l, { task: "tsk2" }, { task: null })).toEqual({ resource: {}, view: {}, push: false });
  });
  test("another resource is another page, pushed, keeping the view", () => {
    expect(lensViewChange(l, { stream_id: 1, days: 14 }, { stream_id: 2 })).toEqual({ resource: { stream_id: 2 }, view: { days: 14 }, push: true });
  });
  test("a filter or a pick in the URI's `?` is misplaced", () => {
    expect(misplacedViewParams(l, { stream_id: 1, days: 14, task: "t" })).toEqual(["days", "task"]);
  });
});

describe("changedParams", () => {
  test("keeps only values that differ from the lens defaults", () => {
    const l = lens({ params: [{ name: "a", label: null, default: 1 }, { name: "b", label: null, default: "x" }] });
    expect(changedParams(l, { a: 1, b: "y" })).toEqual({ b: "y" });
    expect(changedParams(l, {})).toEqual({});
  });

  // What a page writes into its ref: a value is its default by its type,
  // so a bool's echo (0) is its default false, and a key that isn't a
  // lens param (a page's own `uri`) is kept as it is.
  test("compares by type and keeps keys that aren't lens params", () => {
    const l = lens({
      params: [
        { name: "merges", label: null, default: false, type: "bool" },
        { name: "range", label: null, default: "30d", type: "date_range" },
      ],
    });
    expect(changedParams(l, { merges: 0, range: "30d", uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" })).toEqual({ uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" });
    expect(changedParams(l, { merges: true, range: "7d" })).toEqual({ merges: true, range: "7d" });
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
    expect(rowAsk(linked, ["sha", "subject"], ["abc123", "Fix"])).toBe("[oxplow uri oxplow:oxplow/commit?sha=abc123] ");
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
  test("file links can carry a line", () => {
    const ref = cellLinkRef({ kind: "file", from: "path", line: "ln", base: null, head: null }, "path", row, cols);
    expect(ref?.id).toBe("oxplow:oxplow/file?path=src/a.rs#lines=7-7");
    expect(lineOf(ref as TabRef)).toBe(7);
  });
  test("commit and metric links", () => {
    expect(cellLinkRef({ kind: "commit", from: "sha", line: null, base: null, head: null }, "sha", row, cols)?.id).toMatch(/^oxplow:oxplow\/commit\?/);
    expect(cellLinkRef({ kind: "metric", from: "key", line: null, base: null, head: null }, "key", row, cols)?.id).toBe(
      "oxplow:oxplow/metric?metric=oxplow.complexity",
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
      deployment: { kind: "deployed", hash: "0123456789abcdef" },
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
      ["oxplow:oxplow/lens?lens=x/a", "Activity"],
      ["oxplow:oxplow/lens?lens=x/c", "Lenses"],
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
  test("a line's targets column gives each series its target: the last one it states", () => {
    const l = lens({ viz: "line", chart: { x: "at", y: "v", series: "s", label: null, size: null, group: null, targets: "goal" } });
    const out = lineSeries(
      l,
      result(["at", "v", "s", "goal"], [[1, 1, "a", 10], [2, 2, "a", 12], [3, 9, "b", null]]),
    );
    expect(out.map((s) => [s.name, s.target])).toEqual([["a", 12], ["b", null]]);
    // Without the column, no series has one.
    const plain = lineSeries(lens({ viz: "line", chart: { x: "at", y: "v", series: null, label: null, size: null, group: null } }), result(["at", "v"], [[1, 1]]));
    expect(plain[0]!.target).toBeNull();
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
    expect(isPage(ref, "diff")).toBe(true);
    const spec = (r: TabRef) => diffSpecOf(r)!;
    const p = spec(ref);
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
    expect(spec(working).rightVersion).toEqual("working");
    // An effort's change is between two snapshots; those open too.
    const effort = cellLinkRef(
      { kind: "diff-at", from: "path", line: null, base: "base", head: "head" },
      "path",
      ["a", null, "snap:3", "snap:4", null],
      cols,
    )!;
    expect(spec(effort).leftVersion).toEqual("snap:3");
    expect(spec(effort).rightVersion).toEqual("snap:4");
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
    expect(isPage(ref, "duplicate-block")).toBe(true);
    expect(duplicateBlockOf(pagePayload(ref)!.params)).toEqual({
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

test("page links name any ref, which opens where its kind's page says", () => {
  expect(cellLinkRef({ kind: "page", from: null, line: null, base: null, head: null }, "p", ["oxplow:oxplow/work_item?id=tsk3&provider=oxplow"], ["p"])).toBe("oxplow:oxplow/work_item?id=tsk3&provider=oxplow");
  expect(cellLinkRef({ kind: "page", from: null, line: null, base: null, head: null }, "p", ["oxplow:oxplow/page?page=git-dashboard"], ["p"])).toBe("oxplow:oxplow/page?page=git-dashboard");
  expect(cellLinkRef({ kind: "page", from: null, line: null, base: null, head: null }, "p", ["oxplow:oxplow/handoff?id=hnd1"], ["p"])).toBe("oxplow:oxplow/handoff?id=hnd1");
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
    const l = lens({ viz: "timeline", timeline: { at: "at", label: "w", uri: "r" } });
    const out = timelineEntries(
      l,
      result(["at", "w", "r"], [
        ["2026-09-30", "shipped", "oxplow:oxplow/commit?sha=abc"],
        ["2026-09-29", "started", null],
      ]),
    );
    expect(out.map((e) => [e.at, e.label, e.ref])).toEqual([
      ["2026-09-29", "started", null],
      ["2026-09-30", "shipped", "oxplow:oxplow/commit?sha=abc"],
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

  test("codeRows keep rows naming a file and a revision, with their marked lines", () => {
    const l = lens({ viz: "code", code: { path: "p", revision: "r", lines: "l", sync: false } });
    expect(
      codeRows(l, result(["p", "r", "l"], [["x.rs", "git:abc", "40-12"], ["y.rs", "working", null], ["z.rs", null, "1"]])),
    ).toEqual([
      { path: "x.rs", revision: "git:abc", lines: { start: 12, end: 40 }, row: ["x.rs", "git:abc", "40-12"] },
      { path: "y.rs", revision: "working", lines: null, row: ["y.rs", "working", null] },
    ]);
  });

  test("diffRow is the first row's file, two revisions and the line it lands on", () => {
    const l = lens({ viz: "diff", diff: { path: "p", from: "a", to: "b", lines: "l" } });
    expect(diffRow(l, result(["p", "a", "b", "l"], [["x.rs", "git:abc", "working", "7-9"], ["y.rs", "git:abc", "working", null]]))).toEqual({
      path: "x.rs",
      leftVersion: "git:abc",
      rightVersion: "working",
      baseLabel: "abc",
      revealLine: 7,
    });
    expect(diffRow(l, result(["p", "a", "b", "l"], []))).toBeNull();
  });

  test("parseLines reads a line or a range, ends in order", () => {
    expect(parseLines("12")).toEqual({ start: 12, end: 12 });
    expect(parseLines(" 40-12 ")).toEqual({ start: 12, end: 40 });
    expect(parseLines("0-3")).toBeNull();
    expect(parseLines("x")).toBeNull();
    expect(parseLines(null)).toBeNull();
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

test("a column's format reads its value; a value it doesn't read shows as without one", () => {
  expect(formatCell(1536, undefined, "bytes")).toBe("1.5 KB");
  expect(formatCell(0.25, undefined, "percent")).toBe("25%");
  expect(formatCell(1500, undefined, "duration")).not.toBe("1500");
  expect(formatCell("2026-10-17T10:00:00Z", undefined, "relative")).toMatch(/ago|just now|\d/);
  expect(formatCell("plain", undefined, "date")).toBe("plain");
  expect(formatCell(3, undefined, "date")).toBe("3");
});

describe("a column that highlights a param", () => {
  test("the column names the param, and markParts splits text around each case-insensitive match", () => {
    const l = lens({ columns: [{ key: "title", label: null, link: null, unit: null, highlight: "filter" }] });
    expect(displayColumns(l, ["title"])[0]?.highlight).toBe("filter");
    expect(markParts("The Quick quick fox", "quick")).toEqual([
      { text: "The ", mark: false },
      { text: "Quick", mark: true },
      { text: " ", mark: false },
      { text: "quick", mark: true },
      { text: " fox", mark: false },
    ]);
    expect(markParts("abc", "")).toEqual([{ text: "abc", mark: false }]);
    expect(markParts("abc", "z")).toEqual([{ text: "abc", mark: false }]);
    expect(markParts("a.b", ".")).toEqual([
      { text: "a", mark: false },
      { text: ".", mark: true },
      { text: "b", mark: false },
    ]);
  });
});

// A form has no rows of its own: it is never "nothing found".
describe("foldEmpty", () => {
  const part = (viz: string) => ({ run: { lens: { viz }, result: { rows: [] } } as unknown as import("../tauri-bridge/generated/bindings.js").LensRun, error: null });
  test("a form part is shown with no rows; an empty table folds", () => {
    const { shown, empty } = foldEmpty([part("form"), part("table")]);
    expect(shown.map((r) => r.run!.lens.viz)).toEqual(["form"]);
    expect(empty.map((r) => r.run!.lens.viz)).toEqual(["table"]);
  });
});

describe("sparkline and badge formats", () => {
  test("a series reads as its latest value; anything else as it is", () => {
    expect(formatCell("[1,2,3.5]", undefined, "sparkline")).toBe("3.5");
    expect(formatCell("[]", undefined, "sparkline")).toBe("");
    expect(formatCell("not a series", undefined, "sparkline")).toBe("not a series");
    expect(formatCell("ok", undefined, "badge")).toBe("ok");
  });
  test("sparklineValues are the numbers of a JSON array, else null", () => {
    expect(sparklineValues("[1, 2, null, 3]")).toEqual([1, 2, 3]);
    expect(sparklineValues('["4", 5]')).toEqual([4, 5]);
    expect(sparklineValues("{}")).toBeNull();
    expect(sparklineValues("nope")).toBeNull();
    expect(sparklineValues(null)).toBeNull();
    expect(sparklineValues(7)).toBeNull();
  });
  test("a column carries its expand lens", () => {
    const l = lens({ columns: [{ key: "a", label: null, link: null, unit: null, icon: null, tone: null, expand: "x/detail" }] as never });
    expect(displayColumns(l, ["a"])[0]!.expand).toBe("x/detail");
  });
});

test("a composite's parts get its row under its params, a param holding a value winning", () => {
  const run = (sha: SqlCell, rows: SqlCell[][]) =>
    ({ params: { stream_id: 1, sha }, result: { columns: ["change_id", "sha"], rows } }) as unknown as LensRun;
  expect(partParams(run("a", [[7, "b"]]))).toEqual({ stream_id: 1, sha: "a", change_id: 7 });
  expect(partParams(run(null, [[7, "b"]]))).toEqual({ stream_id: 1, sha: "b", change_id: 7 });
  expect(partParams(run(null, []))).toEqual({ stream_id: 1, sha: null });
});
