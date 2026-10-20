import { expect, test } from "bun:test";

import { matchPresetKey, rangeFromPreset, rangeValue, resolveRange } from "./rangePresets.js";

const now = Date.parse("2026-06-25T12:00:00Z");
const DAY = 24 * 60 * 60 * 1000;

test("a preset key is a window ending now, and an unknown one is the default week", () => {
  expect(rangeFromPreset("3d", now)).toEqual({ from: now - 3 * DAY, to: now });
  expect(rangeFromPreset("90d", now)).toEqual({ from: now - 90 * DAY, to: now });
  expect(rangeFromPreset("nope", now)).toEqual(rangeFromPreset("7d", now));
});

test("a range value is a preset key, all, or a..b with either end open", () => {
  expect(resolveRange("7d", now)).toEqual(rangeFromPreset("7d", now));
  expect(resolveRange("all", now)).toBeNull();
  expect(resolveRange("2026-06-01..2026-06-03", now)).toEqual({
    from: Date.parse("2026-06-01"),
    // A date as the end takes in the whole day.
    to: Date.parse("2026-06-03") + DAY - 1,
  });
  expect(resolveRange("2026-06-01T10:00:00.000Z..2026-06-02T10:00:00.000Z", now)).toEqual({
    from: Date.parse("2026-06-01T10:00:00.000Z"),
    to: Date.parse("2026-06-02T10:00:00.000Z"),
  });
  expect(resolveRange("2026-06-20..", now)).toEqual({ from: Date.parse("2026-06-20"), to: now });
  expect(resolveRange("..2026-06-03", now)?.from).toBe(0);
});

test("a window is its preset key when it is one, else a..b", () => {
  expect(matchPresetKey(rangeFromPreset("3d", now), now)).toBe("3d");
  expect(matchPresetKey({ from: now - 99, to: now - 10 * 60 * 1000 }, now)).toBe("custom");
  expect(matchPresetKey({ from: now - 99_999_999, to: now }, now)).toBe("custom");
  expect(rangeValue(rangeFromPreset("30d", now), now)).toBe("30d");
  const brushed = { from: now - 5 * 60 * 1000, to: now - 1000 };
  const value = rangeValue(brushed, now);
  expect(value).toBe(`${new Date(brushed.from).toISOString()}..${new Date(brushed.to).toISOString()}`);
  expect(resolveRange(value, now)).toEqual(brushed);
});
