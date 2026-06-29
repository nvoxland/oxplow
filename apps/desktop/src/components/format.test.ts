import { describe, expect, test } from "bun:test";
import { formatDateOnly, formatTimeOnly, isSameCalendarDay } from "./format.js";

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
