/// Decorators (P6b.C5, experimental): labels from an extension's model on
/// core refs — a chip on a page whose ref the model lists (`ref-chip`), a
/// badge after a lens cell that links to one (`row-badge`). Additive: a
/// page or row is complete without them.
import { parseRef } from "../refs/ref.js";
import type { DecoratorPlacement, Extension, SqlCell, SqlQueryResult, UiDecorator } from "../tauri-bridge/generated/bindings.js";
import type { PageChip } from "../tabs/Page.js";

/** One label on one ref. */
export interface Decoration {
  ref: string;
  label: string;
  color: string | null;
  extension: string;
}

/** The enabled extensions' decorators for `placement`. */
export function decoratorsFor(extensions: Extension[], placement: DecoratorPlacement): UiDecorator[] {
  return extensions.filter((e) => e.enabled).flatMap((e) => e.ui.decorators.filter((d) => d.placement === placement));
}

/** The most decorations one extension adds to one ref (tsk934). */
export const MAX_DECORATIONS_PER_REF = 3;
/** The longest label shown, in characters; a longer one ends in `…`. */
export const MAX_LABEL = 40;
/** The most refs one query asks about; more go in further queries. */
export const REFS_PER_QUERY = 200;

/** One query of a decorator's: its SQL, the refs it binds, and a row
 *  limit with room for each of them. */
export interface DecorationQuery {
  sql: string;
  params: SqlCell[];
  limit: number;
}

/** The queries for `decorator` over the refs of its kind, `REFS_PER_QUERY`
 *  at a time, so a large table's badges aren't cut off by a row limit;
 *  none when no ref is its kind. Column names are identifiers (checked at
 *  load). */
export function decorationQueries(decorator: UiDecorator, refs: string[]): DecorationQuery[] {
  // Named in the SQL as is: plain identifiers only (the loader checks the
  // same; this is where the SQL is built).
  const names = [decorator.view, decorator.label, ...(decorator.color ? [decorator.color] : [])];
  if (!names.every((n) => IDENTIFIER.test(n))) return [];
  const mine = refs.filter((r) => parseRef(r)?.kind === decorator.kind);
  const color = decorator.color ? `, "${decorator.color}" AS color` : "";
  const out: DecorationQuery[] = [];
  for (let at = 0; at < mine.length; at += REFS_PER_QUERY) {
    const chunk = mine.slice(at, at + REFS_PER_QUERY);
    const slots = chunk.map((_, i) => `?${i + 1}`).join(", ");
    out.push({
      sql: `SELECT ref, "${decorator.label}" AS label${color} FROM ${decorator.view} WHERE ref IN (${slots})`,
      params: chunk,
      limit: chunk.length * MAX_DECORATIONS_PER_REF,
    });
  }
  return out;
}

const IDENTIFIER = /^[a-z_][a-z0-9_]*$/;

/** `extension`'s decorations from a query's rows: at most
 *  `MAX_DECORATIONS_PER_REF` on one ref, each label at most `MAX_LABEL`
 *  characters (tsk934). */
export function decorationsFromResult(result: SqlQueryResult, extension: string): Decoration[] {
  const at = (row: SqlCell[], name: string) => {
    const i = result.columns.indexOf(name);
    return i < 0 ? null : row[i] ?? null;
  };
  const perRef = new Map<string, number>();
  const out: Decoration[] = [];
  for (const row of result.rows) {
    const label = at(row, "label");
    if (label === null || label === "") continue;
    const ref = String(at(row, "ref"));
    const count = perRef.get(ref) ?? 0;
    if (count >= MAX_DECORATIONS_PER_REF) continue;
    perRef.set(ref, count + 1);
    out.push({
      ref,
      label: shortened(String(label)),
      color: at(row, "color") == null ? null : String(at(row, "color")),
      extension,
    });
  }
  return out;
}

function shortened(label: string): string {
  const chars = [...label];
  return chars.length <= MAX_LABEL ? label : `${chars.slice(0, MAX_LABEL - 1).join("")}…`;
}

/** A color from extension data, only when it's a plain one: `#rgb[a]` /
 *  `#rrggbb[aa]` or a CSS color name. */
export function safeColor(color: string | null): string | null {
  if (!color) return null;
  return /^#[0-9a-fA-F]{3,8}$|^[a-zA-Z]+$/.test(color) ? color : null;
}

/** `ref`'s decorations as page chips, after the page's own. */
export function chipsFor(decorations: Decoration[], ref: string): PageChip[] {
  return decorations
    .filter((d) => d.ref === ref)
    .map((d) => {
      const color = safeColor(d.color);
      return color ? { label: d.label, color, title: `from ${d.extension}` } : { label: d.label, title: `from ${d.extension}` };
    });
}
