import { describe, expect, it, mock } from "bun:test";

import type { MetricCatalogEntry, SeriesPoint } from "../api.js";
import {
  buildAddMetricMenu,
  buildAddToDashboardMenu,
  deltaTone,
  latestValue,
  parseTileOptions,
  resolveTileWindow,
  tileSpanStyle,
} from "./customDashboardData.js";

function entry(key: string, title: string, category: string | null): MetricCatalogEntry {
  return {
    key,
    title,
    kind: "gauge",
    language: null,
    scope: "built-in",
    enabled: true,
    target: null,
    trigger: "onCapture",
    toggleable: false,
    category,
  };
}

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
});

describe("tileSpanStyle", () => {
  it("maps wide to a column span and tall to a row span", () => {
    expect(tileSpanStyle("wide")).toEqual({ gridColumn: "span 2" });
    expect(tileSpanStyle("tall")).toEqual({ gridRow: "span 2" });
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

describe("buildAddMetricMenu", () => {
  const catalog: MetricCatalogEntry[] = [
    entry("oxplow.coverage.line_pct", "Line coverage", "testing"),
    entry("oxplow.tokens.total", "Tokens", "operational"),
    entry("oxplow.complexity.avg", "Avg complexity", "static-quality"),
    entry("my.custom.metric", "My metric", "custom"),
    entry("weird.one", "Uncategorized", null),
  ];

  it("groups metrics into category submenus in a stable order", () => {
    const menu = buildAddMetricMenu(catalog, () => {});
    // One submenu per non-empty category, ordered operational → testing →
    // static-quality → custom → other.
    expect(menu.map((m) => m.label)).toEqual([
      "Operational",
      "Testing",
      "Static quality",
      "Custom",
      "Other",
    ]);
    for (const group of menu) {
      expect(group.submenu && group.submenu.length).toBeGreaterThan(0);
    }
  });

  it("sorts metrics alphabetically within a category and calls onPick with the key", () => {
    const catalog2 = [
      entry("b.key", "Beta", "testing"),
      entry("a.key", "Alpha", "testing"),
    ];
    const onPick = mock((_key: string) => {});
    const menu = buildAddMetricMenu(catalog2, onPick);
    const testing = menu.find((m) => m.label === "Testing");
    expect(testing?.submenu?.map((i) => i.label)).toEqual(["Alpha", "Beta"]);
    testing?.submenu?.[0]?.run?.();
    expect(onPick).toHaveBeenCalledWith("a.key");
  });

  it("returns an empty menu for an empty catalog", () => {
    expect(buildAddMetricMenu([], () => {})).toEqual([]);
  });
});

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
