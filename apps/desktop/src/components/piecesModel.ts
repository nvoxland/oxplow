/// Settings → Pieces, as data (pure): the capabilities a project chooses an
/// implementation of, read from `v_capability_provider`, with what's active
/// and why, and what a choice writes. See `.context/work-tracking.md`
/// "Swappable pieces".

import type { Extension, SqlCell, SqlQueryResult } from "../tauri-bridge/generated/bindings.js";

export interface PieceChoice {
  id: string;
  title: string;
  /** `builtin`, `external`, `none`, `core`. */
  source: string;
  /** The features it declares that are on. */
  features: string[];
}

export interface Piece {
  capability: string;
  title: string;
  /** It may be `none`. */
  optional: boolean;
  /** Its available implementations, as the registry lists them. */
  choices: PieceChoice[];
  /** The active implementation's id. */
  active: string;
  /** Why: `personal`, `project`, `default` or `fallback`. */
  chosenBy: string | null;
  /** A chosen implementation that isn't available, when it fell back. */
  unavailable: string | null;
}

/** The choosable capabilities in `result` (rows of `v_capability_provider`),
 *  by capability id. */
export function piecesFromResult(result: SqlQueryResult): Piece[] {
  const at = (row: SqlCell[], name: string) => row[result.columns.indexOf(name)] ?? null;
  const pieces = new Map<string, Piece>();
  for (const row of result.rows) {
    if (Number(at(row, "choosable")) !== 1) continue;
    const capability = String(at(row, "capability"));
    const piece =
      pieces.get(capability) ??
      ({
        capability,
        title: String(at(row, "capability_title")),
        optional: Number(at(row, "optional")) === 1,
        choices: [],
        active: "",
        chosenBy: null,
        unavailable: null,
      } satisfies Piece);
    pieces.set(capability, piece);
    const id = String(at(row, "provider"));
    if (Number(at(row, "available")) !== 1) {
      piece.unavailable = id;
      continue;
    }
    let features: string[] = [];
    try {
      const parsed = JSON.parse(String(at(row, "features") ?? "{}")) as Record<string, unknown>;
      features = Object.entries(parsed)
        .filter(([, on]) => on === true)
        .map(([name]) => name);
    } catch {
      features = [];
    }
    piece.choices.push({ id, title: String(at(row, "title") ?? id), source: String(at(row, "source")), features });
    if (Number(at(row, "active")) === 1) {
      piece.active = id;
      piece.chosenBy = at(row, "chosen_by") === null ? null : String(at(row, "chosen_by"));
    }
  }
  return [...pieces.values()].sort((a, b) => a.capability.localeCompare(b.capability));
}

/** Why the active implementation is the one it is, in a person's words. */
export function chosenNote(piece: Piece): string {
  switch (piece.chosenBy) {
    case "personal":
      return "Your own choice.";
    case "project":
      return "The project's choice.";
    case "fallback":
      return `\`${piece.unavailable ?? "?"}\` was chosen but isn't available (its extension is disabled or its instance isn't running), so it's ${piece.active}.`;
    default:
      return "The default.";
  }
}

/** `activeProviders` with `capability` set to `id`, or its entry removed
 *  (`null`: the default, or the same as the project); `null` when nothing
 *  is left, so the key is unset. */
export function nextChoices(
  current: Record<string, string>,
  capability: string,
  id: string | null,
): Record<string, string> | null {
  const { [capability]: _, ...others } = current;
  const next = id === null ? others : { ...others, [capability]: id };
  return Object.keys(next).length === 0 ? null : next;
}

/** What doesn't run without `capability`: the enabled extensions' lenses
 *  and hints that declare they need it (or one of its features). */
export function offWithout(capability: string, extensions: Pick<Extension, "enabled" | "lenses" | "advisories">[]): string[] {
  const needs = (n: string[]) => n.some((need) => need === capability || need.startsWith(`${capability}.`));
  return extensions
    .filter((e) => e.enabled)
    .flatMap((e) => [
      ...e.lenses.filter((l) => needs(l.needs)).map((l) => l.title),
      ...e.advisories.filter((a) => needs(a.needs)).map((a) => `the ${a.id} hint`),
    ]);
}
