// The Tasks board's pure half: what its frame draws from the page's rows,
// with no DOM. The desktop suite tests it (`apps/desktop/src/viz/boardModel.test.ts`).

export type Cell = string | number | boolean | null;
export interface Rows {
  columns: string[];
  rows: Cell[][];
  truncated: boolean;
  error?: string;
}
/** One row, by column. */
export type Rec = Record<string, unknown>;

/** A loader's rows as records; none when it has none. */
export function records(rows: Rows | undefined): Rec[] {
  if (!rows) return [];
  return rows.rows.map((r) => Object.fromEntries(rows.columns.map((c, i) => [c, r[i] ?? null])));
}

/** The board's columns, in order. Canceled items are only on the Task List. */
export const COLUMNS: { state: string; label: string }[] = [
  { state: "todo", label: "To Do" },
  { state: "in_progress", label: "In Progress" },
  { state: "blocked", label: "Blocked" },
  { state: "done", label: "Done" },
];

export interface Column {
  state: string;
  label: string;
  cards: Rec[];
  /** How many items are in this state, shown or not. */
  total: number;
  /** How many the column doesn't show (Done shows the latest few). */
  more: number;
}

/** The columns from the board's rows and the per-state counts. The agent's
 *  live card (no `uri`) leads its column. */
export function boardColumns(board: Rec[], counts: Rec[]): Column[] {
  const n = new Map(counts.map((c) => [String(c.state), Number(c.n) || 0]));
  return COLUMNS.map(({ state, label }) => {
    const here = board.filter((r) => r.state === state);
    const cards = [...here.filter((r) => r.uri == null), ...here.filter((r) => r.uri != null)];
    const items = cards.filter((r) => r.uri != null).length;
    const total = n.get(state) ?? items;
    return { state, label, cards, total, more: Math.max(0, total - items) };
  });
}

export interface Field {
  name: string;
  title: string;
  values: string[];
}

/** The list's own choice fields a person may set (priority, …), from the
 *  `work-item-fields` query's rows. */
export function choiceFields(rows: Rec[]): Field[] {
  return rows
    .filter((r) => r.kind === "enum" && !truthy(r.read_only))
    .map((r) => ({ name: String(r.name), title: String(r.title ?? r.name), values: parseList(r.values) }))
    .filter((f) => f.values.length > 0);
}

/** A command a card runs, as the component's `invoke` takes it. */
export interface Act {
  command: string;
  input: Record<string, unknown>;
}

export type MenuEntry =
  | { kind: "item"; id: string; label: string; act?: Act; open?: string; checked?: boolean; danger?: boolean }
  | { kind: "heading"; label: string }
  | { kind: "separator" };

const MARKS: { state: string; label: string; gate: string }[] = [
  { state: "todo", label: "Mark To Do", gate: "not_todo" },
  { state: "in_progress", label: "Mark In Progress", gate: "not_in_progress" },
  { state: "blocked", label: "Mark Blocked", gate: "not_blocked" },
  { state: "done", label: "Mark Done", gate: "not_done" },
  { state: "canceled", label: "Mark Canceled", gate: "not_canceled" },
];

/** A card's menu: Open, the states it can move to, each choice field's
 *  values, then where it may go and Delete — only what its gates allow.
 *  The agent's live card has none. */
export function cardMenu(card: Rec, fields: Field[]): MenuEntry[] {
  if (card.uri == null) return [];
  const uri = String(card.uri);
  const out: MenuEntry[] = [{ kind: "item", id: "open", label: "Open", open: uri }];
  const group = (entries: MenuEntry[]) => {
    if (entries.length > 0) out.push({ kind: "separator" }, ...entries);
  };
  group(MARKS.filter((m) => truthy(card[m.gate])).map((m) => ({
    kind: "item" as const, id: `to-${m.state}`, label: m.label,
    act: { command: "oxplow.work_item.transition", input: { uri, to: m.state } },
  })));
  const native = parseObject(card.native);
  for (const f of fields) {
    group([
      { kind: "heading", label: f.title },
      ...f.values.map((v) => ({
        kind: "item" as const, id: `${f.name}-${v}`, label: humanize(v), checked: native[f.name] === v,
        act: { command: "oxplow.work_item.update", input: { uri, native: { [f.name]: v } } },
      })),
    ]);
  }
  const moves: MenuEntry[] = [];
  if (truthy(card.has_parent)) moves.push({ kind: "item", id: "unparent", label: "Make top-level", act: { command: "oxplow.work_item.update", input: { uri, parent_uri: "" } } });
  if (truthy(card.can_shelve)) moves.push({ kind: "item", id: "shelve", label: "Send to backlog", act: { command: "oxplow.work_item.move", input: { uri, to: "backlog" } } });
  if (truthy(card.can_bring)) moves.push({ kind: "item", id: "bring", label: "Bring to this thread", act: { command: "oxplow.work_item.move", input: { uri, to: { thread: String(card.thread_ref) } } } });
  if (truthy(card.can_delete)) moves.push({ kind: "item", id: "delete", label: "Delete", danger: true, act: { command: "oxplow.work_item.delete", input: { uri } } });
  group(moves);
  return out;
}

/** What dropping `card` on column `to`, ahead of `before` (null: at the
 *  end), runs: a move to that state, or within its own column a reorder
 *  when the list orders items. Null: nothing to do. */
export function dropAct(card: Rec, to: string, before: Rec | null): Act | null {
  if (card.uri == null) return null;
  const uri = String(card.uri);
  if (to !== card.state) return { command: "oxplow.work_item.transition", input: { uri, to } };
  if (!truthy(card.can_order) || (before && before.uri === card.uri)) return null;
  return { command: "oxplow.work_item.reorder", input: before?.uri ? { uri, before: String(before.uri) } : { uri } };
}

export interface Filters {
  show: string;
  priority: string;
  q: string;
}

/** The board's filters: the tab's view params, else what the page ran
 *  with, else the defaults. */
export function filtersOf(view: Record<string, string> | null, params: Record<string, unknown>): Filters {
  const pick = (k: string, d: string) => {
    const v = view?.[k] ?? params[k];
    return v == null ? d : String(v);
  };
  return { show: pick("show", "thread"), priority: pick("priority", "any"), q: pick("q", "") };
}

/** Task List at the closed items of the board's scope: where Done's
 *  "more" goes. */
export function moreRef(f: Filters): string {
  return `oxplow:oxplow/page?page=task-list#show=${encodeURIComponent(f.show)}&state=done`;
}

function truthy(v: unknown): boolean {
  return v === true || v === 1 || v === "1" || v === "true";
}

function parseList(v: unknown): string[] {
  if (Array.isArray(v)) return v.map(String);
  if (typeof v !== "string") return [];
  try {
    const p = JSON.parse(v);
    return Array.isArray(p) ? p.map(String) : [];
  } catch {
    return [];
  }
}

function parseObject(v: unknown): Record<string, unknown> {
  if (typeof v !== "string") return {};
  try {
    const p = JSON.parse(v);
    return p && typeof p === "object" && !Array.isArray(p) ? p : {};
  } catch {
    return {};
  }
}

function humanize(v: string): string {
  const s = v.replace(/_/g, " ");
  return s.charAt(0).toUpperCase() + s.slice(1);
}
