/**
 * The URI a selection in an editor or a diff names, for Ask the Assistant
 * and Add to Context (`.context/usability.md` "Add to Context").
 */

import { fileLinesUri } from "./uri/uri.js";

/** The URI of a selection's lines (the same ones Ask About This names). */
export function selectionUri(
  path: string,
  sel: { startLineNumber: number; endLineNumber: number; endColumn: number },
  rev: string | null = null,
): string {
  const end = sel.endColumn === 1 && sel.endLineNumber > sel.startLineNumber ? sel.endLineNumber - 1 : sel.endLineNumber;
  return fileLinesUri(path, sel.startLineNumber, end, rev);
}
