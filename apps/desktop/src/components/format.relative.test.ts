import { expect, test } from "bun:test";

import { formatInteger, formatPercent, formatRelative } from "./format.js";

const now = Date.parse("2026-10-17T12:00:00Z");

test("formatRelative says how long ago, then falls back to the date", () => {
  expect(formatRelative("2026-10-17T11:59:30Z", now)).toBe("just now");
  expect(formatRelative("2026-10-17T11:55:00Z", now)).toBe("5 min ago");
  expect(formatRelative("2026-10-17T09:00:00Z", now)).toBe("3 h ago");
  expect(formatRelative("2026-10-15T12:00:00Z", now)).toBe("2 d ago");
  expect(formatRelative("2026-09-01T12:00:00Z", now)).not.toContain("ago");
  expect(formatRelative("not a time", now)).toBe("not a time");
});

test("formatInteger and formatPercent", () => {
  expect(formatInteger(12345.4)).toBe((12345).toLocaleString());
  expect(formatPercent(0.42)).toBe("42%");
  expect(formatPercent(0.042)).toBe("4.2%");
  expect(formatPercent(0)).toBe("0%");
});
