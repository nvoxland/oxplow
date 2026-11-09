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

/** One of the list's stages: its id, its name and its bucket. */
export interface Stage {
  value: string;
  label: string;
  bucket: string;
}

/** The `work-item-stages` query's rows, in the list's order. */
export function stagesOf(rows: Rec[]): Stage[] {
  return rows
    .filter((r) => r.value != null)
    .map((r) => ({ value: String(r.value), label: String(r.label ?? r.value), bucket: String(r.bucket ?? "") }));
}

/** Buckets the board leaves to the Task List. */
const OFF_BOARD = new Set(["backlog", "canceled"]);

export interface Column {
  /** The stage it holds. */
  stage: string;
  bucket: string;
  label: string;
  cards: Rec[];
  /** How many items are in this stage, shown or not. */
  total: number;
  /** How many the column doesn't show (Done shows the latest few). */
  more: number;
}

/** The columns: a stage of the list each, in its order, but its backlog
 *  and canceled ones (the Task List's), from the board's rows and the
 *  per-stage counts. The agent's live card (no `uri`) leads the first
 *  started stage. */
export function boardColumns(board: Rec[], counts: Rec[], stages: Stage[]): Column[] {
  const n = new Map(counts.map((c) => [String(c.stage), Number(c.n) || 0]));
  const started = stages.find((s) => s.bucket === "in_progress")?.value;
  const live = board.filter((r) => r.uri == null);
  return stages
    .filter((s) => !OFF_BOARD.has(s.bucket))
    .map(({ value: stage, label, bucket }) => {
      const cards = [...(stage === started ? live : []), ...board.filter((r) => r.uri != null && r.stage === stage)];
      const items = cards.filter((r) => r.uri != null).length;
      const total = n.get(stage) ?? items;
      return { stage, bucket, label, cards, total, more: Math.max(0, total - items) };
    });
}

/** A run of To Do cards under one epic: its heading (the epic's card when
 *  the epic is in this column too) and its children. The loose cards are
 *  the last group, `uri` null, with no heading. */
export interface EpicGroup {
  uri: string | null;
  title: string | null;
  /** The epic's own card, when it is in this column. */
  epic: Rec | null;
  cards: Rec[];
}

/** `cards` (one column's) under their epics, in the order each first
 *  appears; `all` (every card on the board) says which items are epics. */
export function epicGroups(cards: Rec[], all: Rec[]): EpicGroup[] {
  const epics = new Set(all.map((r) => r.parent_uri).filter((u): u is string => typeof u === "string"));
  const groups = new Map<string, EpicGroup>();
  const loose: Rec[] = [];
  const group = (uri: string, title: string | null) => {
    let g = groups.get(uri);
    if (!g) groups.set(uri, (g = { uri, title, epic: null, cards: [] }));
    return g;
  };
  for (const card of cards) {
    const uri = card.uri == null ? null : String(card.uri);
    if (uri && epics.has(uri)) {
      const g = group(uri, String(card.title ?? ""));
      g.epic = card;
      g.title = String(card.title ?? "");
    } else if (typeof card.parent_uri === "string") {
      group(card.parent_uri, card.parent_title == null ? null : String(card.parent_title)).cards.push(card);
    } else loose.push(card);
  }
  const out = [...groups.values()];
  if (loose.length > 0 || out.length === 0) out.push({ uri: null, title: null, epic: null, cards: loose });
  return out;
}

/** What a compact card says on hover: its title, then its epic, fields,
 *  thread and comments, a line each. */
export function cardHint(card: Rec): string {
  return [card.title, card.parent_title ? `in ${card.parent_title}` : null, card.fields, card.assignee_shown, comments(card.comments)]
    .filter((x) => x !== null && x !== undefined && x !== "")
    .map(String)
    .join("\n");
}

const PRIORITIES = ["urgent", "high", "medium", "low"];

/** A card's priority (the list's `priority` field), for its coloured edge;
 *  null without one the board knows. */
export function priorityOf(card: Rec): string | null {
  const p = parseObject(card.native).priority;
  return typeof p === "string" && PRIORITIES.includes(p) ? p : null;
}

/** The line under a column with more than it shows: a done stage's goes
 *  to its whole history in the Task List. */
export function moreLabel(col: Column): string {
  return col.bucket === "done" ? `All ${col.total} in Task List` : `${col.more} more…`;
}

/** "2 comments", "1 comment", or null. */
export function comments(n: unknown): string | null {
  const k = Number(n) || 0;
  return k === 0 ? null : k === 1 ? "1 comment" : `${k} comments`;
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

/** Someone a card can be assigned to: a thread or the person. */
export interface Assignee {
  uri: string;
  title: string;
}

/** The page's `assignees` rows: the stream's open threads, then the person. */
export function assigneesOf(rows: Rec[]): Assignee[] {
  return rows.filter((r) => r.uri != null).map((r) => ({ uri: String(r.uri), title: String(r.title ?? r.uri) }));
}

/** What a card's menu offers beyond the card itself. */
export interface MenuContext {
  fields: Field[];
  assignees: Assignee[];
  /** Every stage of the list, the backlog's and canceled ones too. */
  stages: Stage[];
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

/** A card's menu: Open, the list's other stages it can move to, each choice field's
 *  values, who it's assigned to (an open card: a thread, the person or
 *  nobody), then Make top-level and Delete — only what its gates allow.
 *  The agent's live card has none. */
export function cardMenu(card: Rec, { fields, assignees, stages }: MenuContext): MenuEntry[] {
  if (card.uri == null) return [];
  const uri = String(card.uri);
  const out: MenuEntry[] = [{ kind: "item", id: "open", label: "Open", open: uri }];
  const group = (entries: MenuEntry[]) => {
    if (entries.length > 0) out.push({ kind: "separator" }, ...entries);
  };
  group(stages.filter((s) => s.value !== card.stage).map((s) => ({
    kind: "item" as const, id: `to-${s.value}`, label: `Mark ${s.label}`,
    act: { command: "oxplow.work_item.transition", input: { uri, to: s.value } },
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
  // A closed card is nobody's: no one to give it to.
  if (truthy(card.is_open)) {
    const assign = (id: string, label: string, to: string | null): MenuEntry => ({
      kind: "item", id, label, checked: (card.assignee ?? null) === to,
      act: { command: "oxplow.work_item.assign", input: { uri, to } },
    });
    group([
      { kind: "heading", label: "Assign to" },
      ...assignees.map((a) => assign(`assign-${a.uri}`, a.title, a.uri)),
      assign("assign-nobody", "Nobody", null),
    ]);
  }
  const moves: MenuEntry[] = [];
  if (truthy(card.has_parent)) moves.push({ kind: "item", id: "unparent", label: "Make top-level", act: { command: "oxplow.work_item.update", input: { uri, parent_uri: "" } } });
  if (truthy(card.can_delete)) moves.push({ kind: "item", id: "delete", label: "Delete", danger: true, act: { command: "oxplow.work_item.delete", input: { uri } } });
  group(moves);
  return out;
}

/** What dropping `card` on the column of stage `to`, ahead of `before`
 *  (null: at the end), runs: a move to that stage, or within its own
 *  column a reorder in its assignee's queue. Null: nothing to do. */
export function dropAct(card: Rec, to: string, before: Rec | null): Act | null {
  if (card.uri == null) return null;
  const uri = String(card.uri);
  if (to !== card.stage) return { command: "oxplow.work_item.transition", input: { uri, to } };
  if (before && before.uri === card.uri) return null;
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
