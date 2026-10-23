import { describe, expect, it, mock } from "bun:test";

import type { SeriesPoint } from "../api.js";
import {
  buildAddToDashboardMenu,
  deltaTone,
  latestValue,
  metricTile,
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

describe("metricTile", () => {
  it("pins a query over the metric's captures, shown as the metric card", () => {
    const tile = metricTile("oxplow.coverage");
    expect(tile.kind).toBe("query");
    expect(tile.display).toBe("metric");
    expect(tile.sql).toContain("metric_grid('capture')");
    expect(tile.sql).toContain("MEASURE('oxplow.coverage')");
    expect(JSON.parse(tile.optionsJson)).toEqual({ metric: "oxplow.coverage", viz: "line" });
  });
});
