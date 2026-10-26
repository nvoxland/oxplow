/** Pure edits of a dashboard's YAML document. Existing nodes are moved,
 * never rebuilt, so their comments, scalar styles and key order survive.
 * The daemon validates and deploys the returned page separately. */
import { Document, isMap, isSeq, parseDocument } from "yaml";

export type DashboardSize = "small" | "wide" | "tall" | "full";
export type DashboardParam = string | number | boolean | null | { param: string } | { from: string };

export interface DashboardTile {
  /** Stable node id, unique among the grid's children. */
  id: string;
  page: string;
  params?: Record<string, DashboardParam>;
  title?: string;
  size?: DashboardSize;
}

function nonempty(value: string, label: string): void {
  if (typeof value !== "string" || !value.trim()) throw new Error(`${label} is required`);
}

function document(source: string) {
  const doc = parseDocument(source);
  if (doc.errors.length) throw new Error(`Invalid page YAML: ${doc.errors[0].message}`);
  if (!isMap(doc.contents)) throw new Error("A page must be a YAML mapping");
  return doc;
}

function dashboard(source: string) {
  const doc = document(source);
  if (doc.get("category") !== "Dashboards") throw new Error("The page must be in the Dashboards category");
  const view = doc.get("view");
  if (!isMap(view) || !isSeq(view.get("grid"))) throw new Error("A dashboard must have a root view.grid sequence");
  for (const kind of ["stack", "switch", "viz", "embed", "slot", "markdown", "component"]) {
    if (view.has(kind)) throw new Error("A dashboard's root view must be a grid");
  }
  const grid = view.get("grid");
  // Narrow again after reading: collection.get() returns unknown.
  if (!isSeq(grid)) throw new Error("A dashboard must have a root view.grid sequence");
  const ids = grid.items.map((item) => {
    if (!isMap(item)) throw new Error("Every dashboard tile must be a node with a stable id");
    const id = item.get("id");
    if (typeof id !== "string" || !id.trim()) throw new Error("Every dashboard tile must have a stable id");
    return id;
  });
  if (new Set(ids).size !== ids.length) throw new Error("Dashboard tile ids must be unique");
  return { doc, grid, ids };
}

function position(index: number, maximum: number): void {
  if (!Number.isInteger(index) || index < 0 || index > maximum) {
    throw new Error(`Tile position must be an integer between 0 and ${maximum}`);
  }
}

function tileIndex(ids: string[], id: string): number {
  const index = ids.indexOf(id);
  if (index < 0) throw new Error(`No dashboard tile with id "${id}"`);
  return index;
}

/** Insert an embedded page at zero-based index, or append when omitted. */
export function addTile(source: string, tile: DashboardTile, index?: number): string {
  const { doc, grid, ids } = dashboard(source);
  nonempty(tile.id, "Tile id");
  nonempty(tile.page, "Embedded page");
  if (ids.includes(tile.id)) throw new Error(`Dashboard already has tile "${tile.id}"`);
  if (tile.size !== undefined && !["small", "wide", "tall", "full"].includes(tile.size)) {
    throw new Error("Unknown dashboard tile size");
  }
  const at = index ?? grid.items.length;
  position(at, grid.items.length);
  const node = doc.createNode({
    id: tile.id,
    ...(tile.title !== undefined ? { title: tile.title } : {}),
    ...(tile.size !== undefined ? { size: tile.size } : {}),
    embed: { page: tile.page, ...(tile.params !== undefined ? { params: tile.params } : {}) },
  });
  grid.items.splice(at, 0, node);
  return doc.toString();
}

export function removeTile(source: string, id: string): string {
  const { doc, grid, ids } = dashboard(source);
  grid.items.splice(tileIndex(ids, id), 1);
  return doc.toString();
}

/** Move id to its zero-based position in the final grid (0..length-1).
 * The target index is not a position in the grid before removal. */
export function moveTile(source: string, id: string, index: number): string {
  const { doc, grid, ids } = dashboard(source);
  const from = tileIndex(ids, id);
  position(index, grid.items.length - 1);
  if (from === index) return source;
  const [node] = grid.items.splice(from, 1);
  grid.items.splice(index, 0, node);
  return doc.toString();
}

/** Change the page's title in place, retaining its scalar comments. */
export function rename(source: string, title: string): string {
  nonempty(title, "Dashboard title");
  const doc = document(source);
  doc.set("title", title.trim());
  return doc.toString();
}

export function newDashboard(title: string): string {
  nonempty(title, "Dashboard title");
  return new Document({
    title: title.trim(),
    icon: "layout-dashboard",
    category: "Dashboards",
    view: { grid: [] },
  }).toString();
}
