import { describe, expect, it, mock } from "bun:test";

import type { SeriesPoint } from "../api.js";
import {
  buildAddToDashboardMenu,
  dashboardBreakoutDims,
  deltaTone,
  parseDashboardSettings,
  latestValue,
  parseTileOptions,
  resolveGroupFilter,
  resolveTileWindow,
  tileSpanStyle,
} from "./customDashboardData.js";

function sample(capturedAt: string, value: number): SeriesPoint {
  return {
    capture_id: 1,
    captured_at: capturedAt,
    value,
    branch: null,
    group: null,
  } as unknown as SeriesPoint;
}

describe("parseTileOptions", () => {
  it("returns an empty object for null / undefined / blank", () => {
    expect(parseTileOptions(null)).toEqual({});
    expect(parseTileOptions(undefined)).toEqual({});
    expect(parseTileOptions("")).toEqual({});
  });

  it("parses a well-formed options blob", () => {
    expect(parseTileOptions('{"viz":"number","mode":"cumulative","scale":"zero","title":"Cov"}')).toEqual({
      viz: "number",
      mode: "cumulative",
      scale: "zero",
      title: "Cov",
    });
  });

  it("ignores malformed JSON instead of throwing", () => {
    expect(parseTileOptions("{not json")).toEqual({});
    // A JSON primitive (not an object) is also ignored.
    expect(parseTileOptions("42")).toEqual({});
  });

  it("drops unrecognized viz / mode / scale values", () => {
    const opts = parseTileOptions('{"viz":"pie","mode":"nonsense","scale":"log","title":123}');
    expect(opts.viz).toBeUndefined();
    expect(opts.mode).toBeUndefined();
    expect(opts.scale).toBeUndefined();
    // A non-string title is dropped too.
    expect(opts.title).toBeUndefined();
  });
});

describe("latestValue", () => {
  it("returns the newest sample's value regardless of input order", () => {
    const samples = [
      sample("2026-07-15T00:00:00Z", 10),
      sample("2026-07-17T00:00:00Z", 30),
      sample("2026-07-16T00:00:00Z", 20),
    ];
    expect(latestValue(samples)).toBe(30);
  });

  it("returns null when there are no parseable samples", () => {
    expect(latestValue([])).toBeNull();
    expect(latestValue([sample("not-a-date", 5)])).toBeNull();
  });
});

describe("parseTileOptions — phase 4 fields", () => {
  it("parses the added viz kinds, size, dim, text, range and branch", () => {
    expect(
      parseTileOptions(
        '{"viz":"sparkline","size":"wide","dim":"package","text":"# Hi","range":"30d","branch":"main"}',
      ),
    ).toEqual({
      viz: "sparkline",
      size: "wide",
      dim: "package",
      text: "# Hi",
      range: "30d",
      branch: "main",
    });
    expect(parseTileOptions('{"viz":"bar"}').viz).toBe("bar");
    expect(parseTileOptions('{"size":"tall"}').size).toBe("tall");
  });

  it("drops an unrecognized size but keeps the rest", () => {
    const opts = parseTileOptions('{"viz":"line","size":"enormous"}');
    expect(opts.viz).toBe("line");
    expect(opts.size).toBeUndefined();
  });

  it("parses the off-target alert toggle, dropping non-booleans", () => {
    expect(parseTileOptions('{"alertOffTarget":false}').alertOffTarget).toBe(false);
    expect(parseTileOptions('{"alertOffTarget":true}').alertOffTarget).toBe(true);
    // Absent means "unset" — the tile applies its own default.
    expect(parseTileOptions("{}").alertOffTarget).toBeUndefined();
    expect(parseTileOptions('{"alertOffTarget":"yes"}').alertOffTarget).toBeUndefined();
  });
});

describe("tileSpanStyle", () => {
  it("maps wide to a column span and tall to a row span", () => {
    expect(tileSpanStyle("wide")).toEqual({ gridColumn: "span 2" });
    expect(tileSpanStyle("tall")).toEqual({ gridRow: "span 2" });
  });

  it("maps full to the whole grid width — the heading-band size", () => {
    expect(tileSpanStyle("full")).toEqual({ gridColumn: "1 / -1" });
  });

  it("gives a small / absent size no span at all", () => {
    expect(tileSpanStyle("small")).toEqual({});
    expect(tileSpanStyle(undefined)).toEqual({});
  });
});

describe("resolveTileWindow", () => {
  const now = Date.parse("2026-07-18T00:00:00Z");
  const dashRange = { from: now - 1000, to: now };

  it("inherits the dashboard's range and branch when the tile overrides nothing", () => {
    expect(resolveTileWindow({}, { range: dashRange, branch: "main" }, now)).toEqual({
      range: dashRange,
      branch: "main",
    });
  });

  it("lets a per-tile range preset win over the dashboard's", () => {
    const got = resolveTileWindow({ range: "1d" }, { range: dashRange, branch: null }, now);
    expect(got.range).not.toBeNull();
    // A "last day" preset spans 24h ending now.
    expect(got.range!.to).toBe(now);
    expect(now - got.range!.from).toBe(24 * 60 * 60 * 1000);
  });

  it("treats a tile range of 'all' as no time window, overriding the dashboard", () => {
    expect(resolveTileWindow({ range: "all" }, { range: dashRange, branch: null }, now).range).toBeNull();
  });

  it("lets a per-tile branch win over the dashboard's", () => {
    expect(resolveTileWindow({ branch: "feature" }, { range: null, branch: "main" }, now).branch).toBe(
      "feature",
    );
  });
});

describe("dashboardBreakoutDims", () => {
  const spec = (key: string, dims: string[] | null) =>
    ({ key, sliceable_dims_json: dims ? JSON.stringify(dims) : null }) as unknown as Parameters<
      typeof dashboardBreakoutDims
    >[0][number];

  it("unions the dimensions across every tile's metric, sorted", () => {
    const dims = dashboardBreakoutDims([
      spec("a", ["language"]),
      spec("b", ["team", "language"]),
      spec("c", null),
    ]);
    // `package` is always available; the declared dims join it, de-duped.
    expect(dims).toEqual(["language", "package", "team"]);
  });

  it("excludes run/time dims that aren't a per-file grain", () => {
    const dims = dashboardBreakoutDims([spec("a", ["branch", "git_version", "language"])]);
    expect(dims).not.toContain("branch");
    expect(dims).not.toContain("git_version");
    expect(dims).toContain("language");
  });

  it("returns just package when no metric declares extra dims, and nothing for no metrics", () => {
    expect(dashboardBreakoutDims([spec("a", null)])).toEqual(["package"]);
    expect(dashboardBreakoutDims([])).toEqual([]);
  });
});

describe("parseDashboardSettings", () => {
  it("returns an empty object for null / blank / malformed JSON", () => {
    expect(parseDashboardSettings(null)).toEqual({});
    expect(parseDashboardSettings("")).toEqual({});
    expect(parseDashboardSettings("{nope")).toEqual({});
    expect(parseDashboardSettings("7")).toEqual({});
  });

  it("parses a saved view", () => {
    expect(
      parseDashboardSettings('{"range":"7d","branch":"main","filterDim":"package","filterValue":"core"}'),
    ).toEqual({ range: "7d", branch: "main", filterDim: "package", filterValue: "core" });
  });

  it("drops non-string fields rather than seeding the filter row with junk", () => {
    expect(parseDashboardSettings('{"range":5,"branch":true,"filterDim":{},"filterValue":[]}')).toEqual({});
  });
});

describe("resolveGroupFilter", () => {
  const groups = (values: string[], loaded = true) => ({ loaded, values });

  it("is inactive when no dimension is selected", () => {
    expect(resolveGroupFilter(null, null, ["package"], groups(["a"]))).toEqual({
      filtered: false,
      notApplicable: false,
    });
  });

  it("is inactive (not dimmed) when a dimension is chosen but no value yet", () => {
    // The tile keeps showing everything until the user narrows to a value.
    expect(resolveGroupFilter("package", null, ["package"], groups(["a"]))).toEqual({
      filtered: false,
      notApplicable: false,
    });
  });

  it("filters when the metric declares the dimension and has that value", () => {
    expect(resolveGroupFilter("package", "core", ["package"], groups(["core", "ui"]))).toEqual({
      filtered: true,
      notApplicable: false,
    });
  });

  it("is not-applicable when the metric doesn't declare the dimension", () => {
    expect(resolveGroupFilter("language", "rust", ["package"], groups([]))).toEqual({
      filtered: false,
      notApplicable: true,
    });
  });

  it("is not-applicable when the metric declares the dimension but has no data for that value", () => {
    expect(resolveGroupFilter("package", "core", ["package"], groups(["ui", "api"]))).toEqual({
      filtered: false,
      notApplicable: true,
    });
  });

  it("does not flash not-applicable while the groups are still loading", () => {
    expect(resolveGroupFilter("package", "core", ["package"], groups([], false))).toEqual({
      filtered: true,
      notApplicable: false,
    });
  });
});

describe("deltaTone", () => {
  it("reads the sign against the metric's preferred direction", () => {
    // higher-better: up is good, down is bad.
    expect(deltaTone(5, "higher-better")).toBe("good");
    expect(deltaTone(-5, "higher-better")).toBe("bad");
    // lower-better: down is good, up is bad.
    expect(deltaTone(-5, "lower-better")).toBe("good");
    expect(deltaTone(5, "lower-better")).toBe("bad");
  });

  it("is neutral for a zero delta or a neutral/unknown direction", () => {
    expect(deltaTone(0, "higher-better")).toBe("neutral");
    expect(deltaTone(5, "neutral")).toBe("neutral");
    expect(deltaTone(5, "whatever")).toBe("neutral");
  });
});

// NB: the add-metric picker's sectioning + search now live in
// `components/Dashboard/metricPicker.ts` (backed by the canonical
// `buildMetricSections`) and are covered by `metricPicker.test.ts`.

describe("buildAddToDashboardMenu", () => {
  const dashboards = [
    { id: "dsh1", title: "Coverage", sort_index: 0 },
    { id: "dsh2", title: "Tokens", sort_index: 1 },
  ] as unknown as Parameters<typeof buildAddToDashboardMenu>[0];

  it("lists one entry per dashboard, then a separator and 'New dashboard…'", () => {
    const menu = buildAddToDashboardMenu(dashboards, () => {}, () => {});
    expect(menu.map((m) => m.label)).toEqual(["Coverage", "Tokens", "", "New dashboard…"]);
    expect(menu[2]?.separator).toBe(true);
  });

  it("calls onPick with the chosen dashboard id, and onNew for the new entry", () => {
    const onPick = mock((_id: string) => {});
    const onNew = mock(() => {});
    const menu = buildAddToDashboardMenu(dashboards, onPick, onNew);
    menu[1]?.run?.();
    expect(onPick).toHaveBeenCalledWith("dsh2");
    menu[3]?.run?.();
    expect(onNew).toHaveBeenCalled();
  });

  it("offers only 'New dashboard…' (no leading separator) when there are none yet", () => {
    const menu = buildAddToDashboardMenu([], () => {}, () => {});
    expect(menu.map((m) => m.label)).toEqual(["New dashboard…"]);
  });
});
