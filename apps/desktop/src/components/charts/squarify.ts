/** Squarified treemap layout, shared by treemap lenses and change analysis. */

export interface SquarifyInput<T> {
  value: number;
  payload: T;
}
export interface SquarifyOutput<T> {
  payload: T;
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * Squarified treemap (Bruls, Huijsen, Van Wijk 2000) generalized
 * to lay out into any rectangle. Sorts items by value desc, then
 * greedily packs rows whose worst aspect ratio doesn't degrade.
 */
export function squarify<T>(
  items: SquarifyInput<T>[],
  x0: number,
  y0: number,
  width: number,
  height: number,
): SquarifyOutput<T>[] {
  if (items.length === 0 || width <= 0 || height <= 0) return [];
  const sorted = [...items].sort((a, b) => b.value - a.value);
  const totalValue = sorted.reduce((acc, i) => acc + i.value, 0);
  if (totalValue <= 0) return [];
  const totalArea = width * height;
  const scaled = sorted.map((i) => ({
    payload: i.payload,
    area: (i.value / totalValue) * totalArea,
  }));

  const out: SquarifyOutput<T>[] = [];
  let x = x0;
  let y = y0;
  let w = width;
  let h = height;
  let queue = scaled;

  while (queue.length > 0) {
    const shorter = Math.min(w, h);
    const row: typeof queue = [queue[0]!];
    queue = queue.slice(1);
    while (queue.length > 0) {
      const candidate = [...row, queue[0]!];
      if (worstRatio(candidate, shorter) <= worstRatio(row, shorter)) {
        row.push(queue[0]!);
        queue = queue.slice(1);
      } else {
        break;
      }
    }
    const rowTotal = row.reduce((acc, r) => acc + r.area, 0);
    const rowExtent = rowTotal / shorter;
    if (w >= h) {
      let cy = y;
      for (const r of row) {
        const cellH = r.area / rowExtent;
        out.push({ payload: r.payload, x, y: cy, w: rowExtent, h: cellH });
        cy += cellH;
      }
      x += rowExtent;
      w -= rowExtent;
    } else {
      let cx = x;
      for (const r of row) {
        const cellW = r.area / rowExtent;
        out.push({ payload: r.payload, x: cx, y, w: cellW, h: rowExtent });
        cx += cellW;
      }
      y += rowExtent;
      h -= rowExtent;
    }
  }
  return out;
}

/** Worst aspect ratio if `row` is laid along edge of length `shorter`. */
function worstRatio(row: { area: number }[], shorter: number): number {
  const s = row.reduce((acc, r) => acc + r.area, 0);
  if (s === 0) return Number.POSITIVE_INFINITY;
  let max = 0;
  let min = Number.POSITIVE_INFINITY;
  for (const r of row) {
    if (r.area > max) max = r.area;
    if (r.area < min) min = r.area;
  }
  const ss = s * s;
  const w2 = shorter * shorter;
  return Math.max((w2 * max) / ss, ss / (w2 * min));
}
