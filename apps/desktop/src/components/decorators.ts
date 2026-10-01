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

/** The query for `decorator` over the refs of its kind; `null` when none
 *  of `refs` is. Column names are identifiers (checked at load). */
export function decorationQuery(decorator: UiDecorator, refs: string[]): { sql: string; params: SqlCell[] } | null {
  const mine = refs.filter((r) => parseRef(r)?.kind === decorator.kind);
  if (mine.length === 0) return null;
  const color = decorator.color ? `, "${decorator.color}" AS color` : "";
  const slots = mine.map((_, i) => `?${i + 1}`).join(", ");
  return {
    sql: `SELECT ref, "${decorator.label}" AS label${color} FROM ${decorator.view} WHERE ref IN (${slots})`,
    params: mine,
  };
}

export function decorationsFromResult(result: SqlQueryResult, extension: string): Decoration[] {
  const at = (row: SqlCell[], name: string) => {
    const i = result.columns.indexOf(name);
    return i < 0 ? null : row[i] ?? null;
  };
  return result.rows
    .filter((row) => at(row, "label") !== null && at(row, "label") !== "")
    .map((row) => ({
      ref: String(at(row, "ref")),
      label: String(at(row, "label")),
      color: at(row, "color") == null ? null : String(at(row, "color")),
      extension,
    }));
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
