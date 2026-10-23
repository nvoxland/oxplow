import { describe, expect, it, mock } from "bun:test";

import type { SeriesPoint } from "../api.js";
import {
  deltaTone,
  latestValue,
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
