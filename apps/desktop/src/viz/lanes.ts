/// `format: lanes`: the lanes of the graph a list's or table's rows form, as
/// `git log --graph` keeps them. Each row is an id and its parents' ids
/// (first parent first); the rows are laid out in the order they are drawn,
/// each child before its parents. A row's lanes are what comes into its row
/// from above (`incoming`), what leaves it downward (`outgoing`), the lane
/// its dot is in (`column`) and the lane each of its parents' edges goes to.

import type { SqlCell } from "../tauri-bridge/generated/bindings.js";

export interface LaneNode {
  id: string;
  parents: string[];
}

export interface LaneRow {
  /** The lane its dot is in. */
  column: number;
  /** Its lane came from a row above (a child's edge), else it starts here. */
  fromAbove: boolean;
  /** Each lane coming in from above: the id it waits for, or null. */
  incoming: (string | null)[];
  /** Each lane leaving downward: the id it waits for, or null. */
  outgoing: (string | null)[];
  /** The lane each parent's edge goes to, in `parents`' order. */
  edges: number[];
}

/** The rows' lanes and how many lanes the widest row has. Each lane waits
 *  for an id; a row takes its lane over for its first parent, and its other
 *  parents open new lanes. A parent the rows don't hold opens no lane. */
export function laneLayout(nodes: LaneNode[]): { rows: LaneRow[]; width: number } {
  const present = new Set(nodes.map((n) => n.id));
  const lanes: (string | null)[] = [];
  let width = 0;
  const rows = nodes.map((node) => {
    let column = lanes.indexOf(node.id);
    const fromAbove = column !== -1;
    if (!fromAbove) {
      column = lanes.indexOf(null);
      if (column === -1) {
        column = lanes.length;
        lanes.push(null);
      }
      lanes[column] = node.id;
    }
    const incoming = lanes.slice();
    for (let k = 0; k < lanes.length; k++) if (lanes[k] === node.id) lanes[k] = null;
    const edges = node.parents.map((parent, i) => {
      let to = lanes.indexOf(parent);
      if (to === -1) {
        if (i === 0 && lanes[column] === null) to = column;
        else {
          to = lanes.indexOf(null);
          if (to === -1) {
            to = lanes.length;
            lanes.push(null);
          }
        }
        if (present.has(parent)) lanes[to] = parent;
      }
      return to;
    });
    while (lanes.length > 0 && lanes[lanes.length - 1] === null) lanes.pop();
    const outgoing = lanes.slice();
    width = Math.max(width, incoming.length, outgoing.length);
    return { column, fromAbove, incoming, outgoing, edges };
  });
  return { rows, width };
}

/** A `parents` cell: a JSON array of ids; anything else holds none. */
export function parentsOf(cell: SqlCell | undefined): string[] {
  if (typeof cell !== "string" || cell === "") return [];
  try {
    const parsed: unknown = JSON.parse(cell);
    return Array.isArray(parsed) ? parsed.filter((v) => v !== null).map(String) : [];
  } catch {
    return [];
  }
}
