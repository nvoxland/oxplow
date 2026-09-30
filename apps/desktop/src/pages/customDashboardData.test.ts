import { describe, expect, it, mock } from "bun:test";

import type { SeriesPoint } from "../api.js";
import {
  buildAddToDashboardMenu,
  deltaTone,
  latestValue,
  metricTile,
  parseTileOptions,
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
    expect(parseTileOptions('{"viz":"number","title":"Cov"}')).toEqual({ viz: "number", title: "Cov" });
  });

  it("ignores malformed JSON instead of throwing", () => {
    expect(parseTileOptions("{not json")).toEqual({});
    // A JSON primitive (not an object) is also ignored.
    expect(parseTileOptions("42")).toEqual({});
  });

  it("drops unrecognized viz values and non-string titles", () => {
    const opts = parseTileOptions('{"viz":"pie","title":123}');
    expect(opts.viz).toBeUndefined();
    expect(opts.title).toBeUndefined();
  });

  it("reads tiles saved with the retired options as plain line tiles", () => {
    // sparkline / bar tiles, chart mode, scale, breakdown dim and the
    // off-target toggle were removed; old blobs keep what still applies.
    expect(
      parseTileOptions(
        '{"viz":"sparkline","mode":"cumulative","scale":"zero","dim":"package","alertOffTarget":true,"title":"T"}',
      ),
    ).toEqual({ title: "T" });
    expect(parseTileOptions('{"viz":"bar"}')).toEqual({});
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

describe("parseTileOptions — layout and window fields", () => {
  it("parses size, text, range and branch", () => {
    expect(parseTileOptions('{"size":"wide","text":"# Hi","range":"30d","branch":"main"}')).toEqual({
      size: "wide",
      text: "# Hi",
      range: "30d",
      branch: "main",
    });
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

describe("parseTileOptions lens tiles", () => {
  it("keeps a lens tile's lensId", () => {
    expect(parseTileOptions('{"lensId":"mine/threads","size":"wide"}')).toEqual({ lensId: "mine/threads", size: "wide" });
  });
});

describe("query tiles (P4.7)", () => {
  it("parses a query tile's sql, display and metric", () => {
    expect(
      parseTileOptions('{"sql":"SELECT 1","display":"metric","metric":"oxplow.coverage","viz":"number"}'),
    ).toEqual({ sql: "SELECT 1", display: "metric", metric: "oxplow.coverage", viz: "number" });
    expect(parseTileOptions('{"sql":7,"display":null,"metric":[]}')).toEqual({});
  });

  it("metricTile pins a query over the metric's captures, shown as the metric card", () => {
    const tile = metricTile("oxplow.coverage");
    expect(tile.kind).toBe("query");
    expect(tile.display).toBe("metric");
    expect(tile.sql).toContain("metric_grid('capture')");
    expect(tile.sql).toContain("MEASURE('oxplow.coverage')");
    expect(parseTileOptions(tile.optionsJson)).toEqual({ metric: "oxplow.coverage", viz: "line" });
  });
});
