/**
 * What a person does to a viz's rows besides clicking a link: marks them
 * (Cmd/Ctrl+click toggles, Shift+click ranges, Space toggles, Escape
 * clears), drops dragged rows somewhere (a drag action, `drag: true`, runs
 * with where they landed), and runs a row action from its key (`key:`).
 * Pure: `RowsBody` connects the events to these. See
 * `.context/usability.md` "Rows: marks, drags and keys".
 */
import type { PageAction, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { actionOffered } from "./vizActions.js";

/** A row's identity while it is shown: its cells. A row whose cells
 *  change (its state moved) is another row, so its mark goes. */
export function rowKey(row: SqlCell[]): string {
  return JSON.stringify(row);
}

/** The marks after a click on row `index` (of `keys`, the rows in drawing
 *  order), from `anchor` (the row a range starts at): Cmd/Ctrl toggles
 *  the row, Shift marks the range from the anchor, a plain click clears
 *  them. The new anchor is the clicked row, except after a range. */
export function marksAfterClick(
  marks: readonly string[],
  keys: readonly string[],
  index: number,
  anchor: number | null,
  how: { toggle: boolean; range: boolean },
): { marks: string[]; anchor: number | null } {
  const key = keys[index];
  if (key === undefined) return { marks: [...marks], anchor };
  if (how.range && anchor !== null) {
    const [from, to] = anchor <= index ? [anchor, index] : [index, anchor];
    const range = keys.slice(from, to + 1);
    return { marks: [...new Set([...marks, ...range])], anchor };
  }
  if (how.toggle) {
    return { marks: marks.includes(key) ? marks.filter((k) => k !== key) : [...marks, key], anchor: index };
  }
  return { marks: [], anchor: index };
}

/** The marks, in drawing order, that are still shown (a row whose cells
 *  changed is gone from them). */
export function shownMarks(marks: readonly string[], keys: readonly string[]): string[] {
  const set = new Set(marks);
  return keys.filter((k) => set.has(k));
}

/** Where on a row a drop lands, from the pointer's height in it: its top
 *  half is before it, its bottom half after; with `onto`, its middle
 *  third is onto it. */
export function dropZone(y: number, top: number, height: number, onto: boolean): "before" | "after" | "onto" {
  const at = height > 0 ? (y - top) / height : 0;
  if (onto && at > 1 / 3 && at < 2 / 3) return "onto";
  return at < 0.5 ? "before" : "after";
}

/** Where dragged rows landed, as the server binds it: the row below the
 *  drop (`before`), the row above (`after`), the row dropped on, the
 *  group dropped in, and every row of the result in its order after the
 *  drop (`withOrder`). */
export interface DropAt {
  before: Record<string, SqlCell> | null;
  after: Record<string, SqlCell> | null;
  onto: Record<string, SqlCell> | null;
  group: SqlCell | null;
  order: Record<string, SqlCell>[] | null;
}

export const NOWHERE: DropAt = { before: null, after: null, onto: null, group: null, order: null };

/** A drop beside row `index` of `rows` (a group's rows, in drawing order;
 *  the dragged rows left out by the caller): `before` that row or `after`
 *  it, its neighbour on the other side if it has one; or onto it. */
export function dropBeside(
  rows: readonly Record<string, SqlCell>[],
  index: number,
  zone: "before" | "after" | "onto",
  group: SqlCell | null,
): DropAt {
  const at = rows[index] ?? null;
  if (zone === "onto") return { ...NOWHERE, onto: at, group };
  if (zone === "before") return { ...NOWHERE, before: at, after: rows[index - 1] ?? null, group };
  return { ...NOWHERE, before: rows[index + 1] ?? null, after: at, group };
}

/** `at` with its order: `rows` (every row of the result, in order) with
 *  the `dragged` ones moved before `at.before`, else after `at.after`,
 *  else to the end. A drop onto a row moves nothing. */
export function withOrder(
  at: DropAt,
  rows: readonly Record<string, SqlCell>[],
  dragged: readonly Record<string, SqlCell>[],
): DropAt {
  if (at.onto !== null) return { ...at, order: [...rows] };
  const key = (r: Record<string, SqlCell>) => JSON.stringify(r);
  const away = new Set(dragged.map(key));
  const rest = rows.filter((r) => !away.has(key(r)));
  const index = (r: Record<string, SqlCell> | null) => (r ? rest.findIndex((x) => key(x) === key(r)) : -1);
  const before = index(at.before);
  const after = index(at.after);
  const to = before !== -1 ? before : after !== -1 ? after + 1 : rest.length;
  return { ...at, order: [...rest.slice(0, to), ...dragged, ...rest.slice(to)] };
}

/** A drop in a group but on no row (its heading, a board column's empty
 *  space): at its end, after its last row. */
export function dropInGroup(rows: readonly Record<string, SqlCell>[], group: SqlCell | null): DropAt {
  return { ...NOWHERE, after: rows[rows.length - 1] ?? null, group };
}

/** The scopes an action's input names (`{{onto.x}}` → `onto`). */
function scopesOf(input: unknown, out: Set<string> = new Set()): Set<string> {
  if (typeof input === "string") {
    for (const m of input.matchAll(/\{\{\s*([a-z_]+)\.[^}]*\}\}/g)) out.add(m[1]!);
  } else if (Array.isArray(input)) {
    for (const v of input) scopesOf(v, out);
  } else if (input && typeof input === "object") {
    for (const v of Object.values(input)) scopesOf(v, out);
  }
  return out;
}

/** The drag action a drop of `row` at `at` runs: the first the row is
 *  offered (`when`) whose places are all there — one that names the group
 *  needs a group, one that names `onto` needs a row dropped on, one that
 *  names the order needs it. Before and after may be missing (a drop at
 *  an end): the server leaves them out. */
export function dragActionFor(actions: readonly PageAction[], row: Record<string, SqlCell>, at: DropAt): PageAction | null {
  return (
    actions.find((a) => {
      if (!a.drag || !actionOffered(a, row)) return false;
      const scopes = scopesOf(a.input);
      if (scopes.has("group") && at.group === null) return false;
      if (scopes.has("onto") && at.onto === null) return false;
      if (scopes.has("order") && at.order === null) return false;
      if (!scopes.has("onto") && at.onto !== null) return false;
      return true;
    }) ?? null
  );
}

/** Whether a drop at `at` takes `onto` at all: some drag action names it. */
export function takesOnto(actions: readonly PageAction[]): boolean {
  return actions.some((a) => a.drag && scopesOf(a.input).has("onto"));
}

/** Several rows dropped together, in drawing order: each lands after the
 *  one dropped before it, so they keep their order where they land, and
 *  all share one order. A drop onto a row puts each onto it. */
export function chainedDrops(at: DropAt, dragged: readonly Record<string, SqlCell>[]): DropAt[] {
  return dragged.map((_, i) => (i === 0 || at.onto !== null ? at : { ...at, after: dragged[i - 1]! }));
}

/** The row action key `key` runs (`key:`), if any. */
export function keyAction(actions: readonly PageAction[], key: string): PageAction | null {
  return actions.find((a) => a.row && a.key !== null && a.key !== undefined && a.key === key) ?? null;
}
