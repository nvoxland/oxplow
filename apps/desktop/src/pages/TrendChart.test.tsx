import { expect, test } from "bun:test";
import { render } from "@testing-library/react";

import { TrendChart } from "./MetricDetail.js";

// Two points a day apart, so the time axis has a real span to label.
const T0 = Date.parse("2026-07-16T09:00:00Z");
const DAY = 24 * 60 * 60 * 1000;
const POINTS = [
  { t: T0, v: 10 },
  { t: T0 + DAY, v: 20 },
];

function axisLabels(container: HTMLElement): string[] {
  return [...container.querySelectorAll("text")].map((n) => n.textContent ?? "");
}

/**
 * Regression guard for tsk144: a local range-adaptive **y**-tick formatter was
 * named `fmtTick`, shadowing the module-level **time** formatter of the same
 * name, so the x axis rendered raw epoch milliseconds ("1752...") instead of a
 * date. Pure-helper tests couldn't see it — the bug was which function the JSX
 * called — so this asserts on the rendered SVG text.
 */
test("the time axis renders date labels, not raw epoch milliseconds", () => {
  const { container } = render(<TrendChart points={POINTS} />);
  const labels = axisLabels(container);
  expect(labels.length).toBeGreaterThan(0);
  // No label may be a bare epoch-sized integer.
  for (const label of labels) {
    expect(/^\d{10,}$/.test(label.trim())).toBe(false);
  }
  // And at least one label should look like a formatted date/time.
  expect(labels.some((l) => /\d+[/-]\d+/.test(l) || /:\d{2}/.test(l))).toBe(true);
});

test("y-axis labels keep enough precision for a narrow value range", () => {
  // A tight span (1.94–1.97) must not collapse to "1.9"/"2.0".
  const tight = [
    { t: T0, v: 1.94 },
    { t: T0 + DAY, v: 1.97 },
  ];
  const { container } = render(<TrendChart points={tight} />);
  // The padded span is ~0.035, so the adaptive precision gives 2+ decimals.
  expect(axisLabels(container).some((l) => /^\d\.\d{2,}$/.test(l.trim()))).toBe(true);
});

test("a compact chart thins the time axis to its two endpoints", () => {
  const wide = render(<TrendChart points={POINTS} />);
  const compact = render(<TrendChart points={POINTS} width={400} height={200} />);
  const timeish = (c: HTMLElement) =>
    axisLabels(c).filter((l) => /\d+[/-]\d+/.test(l) || /:\d{2}/.test(l));
  // Default (760-wide) gets 4 time ticks; a tile-sized chart gets 2.
  expect(timeish(wide.container).length).toBeGreaterThan(timeish(compact.container).length);
});
