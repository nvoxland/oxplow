import { describe, expect, test } from "bun:test";
import {
  formatDateOnly, formatTimeOnly, isSameCalendarDay,
  formatMetricValue,
  formatMetricValueExact,
  formatDurationMs,
} from "./format.js";

describe("formatDateOnly", () => {
  test("returns a non-empty date label for a valid timestamp", () => {
    expect(formatDateOnly("2026-06-27T03:00:00").length).toBeGreaterThan(0);
  });
  test("falls back to the raw string when unparseable", () => {
    expect(formatDateOnly("not a date")).toBe("not a date");
  });
});

describe("isSameCalendarDay", () => {
  test("true for two times on the same local day", () => {
    expect(isSameCalendarDay("2026-06-27T03:00:00", "2026-06-27T20:30:00")).toBe(true);
  });
  test("false across a day boundary", () => {
    expect(isSameCalendarDay("2026-06-27T23:59:00", "2026-06-28T00:01:00")).toBe(false);
  });
  test("false when either timestamp is unparseable", () => {
    expect(isSameCalendarDay("nope", "2026-06-27T03:00:00")).toBe(false);
    expect(isSameCalendarDay("2026-06-27T03:00:00", "")).toBe(false);
  });
});

describe("formatTimeOnly", () => {
  test("returns a non-empty time label for a valid timestamp", () => {
    expect(formatTimeOnly("2026-06-27T03:00:00").length).toBeGreaterThan(0);
  });
  test("falls back to the raw string when unparseable", () => {
    expect(formatTimeOnly("not a date")).toBe("not a date");
  });
});

describe("formatMetricValue (tsk114)", () => {
  // Expectations are computed through the SAME Intl configs the formatter
  // uses, so assertions hold under any OS locale the test host runs with —
  // what's under test is the ROUTING (unit branches, compact threshold),
  // not Intl itself.
  const grouped = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });
  const compact = new Intl.NumberFormat(undefined, {
    notation: "compact",
    maximumFractionDigits: 1,
  });

  test("plain counts group with the locale's separators", () => {
    expect(formatMetricValue(1234)).toBe(grouped.format(1234));
  });
  test("large counts compact, with the exact form for hovers", () => {
    expect(formatMetricValue(240417)).toBe(compact.format(240417));
    expect(formatMetricValueExact(240417)).toBe(
      new Intl.NumberFormat(undefined, { maximumFractionDigits: 4 }).format(240417),
    );
  });
  test("integers render clean", () => {
    expect(formatMetricValue(5)).toBe(grouped.format(5));
  });
  test("percent unit renders one decimal with the sign attached", () => {
    expect(formatMetricValue(87.444, "%").endsWith("%")).toBe(true);
    expect(formatMetricValue(87.444, "%")).toBe(
      `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 }).format(87.444)}%`,
    );
  });
  test("ms unit humanizes into duration tiers", () => {
    expect(formatMetricValue(850, "ms").endsWith(" ms")).toBe(true);
    expect(formatMetricValue(1234, "ms").endsWith(" s")).toBe(true);
    expect(formatMetricValue(200_000, "ms").endsWith(" m")).toBe(true);
    expect(formatDurationMs(7_200_000).endsWith(" h")).toBe(true);
  });
  test("other units append after the locale-formatted number", () => {
    expect(formatMetricValue(1234, "lines")).toBe(`${grouped.format(1234)} lines`);
  });
  test("non-finite values pass through instead of formatting garbage", () => {
    expect(formatMetricValue(Number.NaN)).toBe("NaN");
  });
});
