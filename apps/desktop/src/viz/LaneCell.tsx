/// One row's part of a `format: lanes` column: the lanes passing by, the
/// row's dot in its lane, and the edges to its parents. It fills its cell's
/// whole height, so a wrapped row's lanes still meet the next row's.
import type { CSSProperties } from "react";

import type { LaneRow } from "./lanes.js";

const LANE = 14; // px per lane
const PAD = 8; // px left and right of the lanes
const RADIUS = 4;
/** The dot's height from the cell's top: the middle of the row's first line
 *  of text, under the cell padding the other cells have. */
const DOT = 18;
/** How far down an edge to another lane slants before it runs straight. */
const SLANT = 16;
const COLORS = 8; // the theme's chart series, --chart-1 … --chart-8

const color = (lane: number) => `var(--chart-${(lane % COLORS) + 1})`;

/** How wide the column is for lanes `width` wide. */
export function lanesWidth(width: number): number {
  return PAD * 2 + LANE * Math.max(1, width);
}

export function LaneCell({ row, width }: { row: LaneRow; width: number }) {
  const x = (k: number) => PAD + k * LANE + LANE / 2;
  const id = row.incoming[row.column] ?? null;
  const lines: { k: number; x1: number; y1: number | string; x2: number; y2: number | string }[] = [];
  row.incoming.forEach((waits, k) => {
    if (waits === null || k === row.column) return;
    // A lane waiting for this row joins its dot; any other passes by.
    if (waits === id) lines.push({ k, x1: x(k), y1: 0, x2: x(row.column), y2: DOT });
    else lines.push({ k, x1: x(k), y1: 0, x2: x(k), y2: "100%" });
  });
  if (row.fromAbove) lines.push({ k: row.column, x1: x(row.column), y1: 0, x2: x(row.column), y2: DOT });
  if (row.outgoing[row.column]) lines.push({ k: row.column, x1: x(row.column), y1: DOT, x2: x(row.column), y2: "100%" });
  for (const to of row.edges) {
    if (to === row.column) continue;
    lines.push({ k: to, x1: x(row.column), y1: DOT, x2: x(to), y2: DOT + SLANT });
    lines.push({ k: to, x1: x(to), y1: DOT + SLANT, x2: x(to), y2: "100%" });
  }
  const w = lanesWidth(width);
  return (
    <svg data-testid="viz-lanes" aria-hidden="true" width={w} style={{ ...svgStyle, width: w }}>
      {lines.map((l, i) => (
        <line key={i} x1={l.x1} y1={l.y1} x2={l.x2} y2={l.y2} stroke={color(l.k)} strokeWidth={1.5} strokeLinecap="round" />
      ))}
      <circle data-lane={row.column} cx={x(row.column)} cy={DOT} r={RADIUS} fill={color(row.column)} stroke={color(row.column)} strokeWidth={1.5} />
    </svg>
  );
}

const svgStyle: CSSProperties = { position: "absolute", top: 0, left: 0, height: "100%", overflow: "visible" };
