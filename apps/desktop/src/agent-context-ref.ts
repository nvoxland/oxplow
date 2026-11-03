/**
 * The URI a selection in an editor or a diff names, for Ask the Assistant
 * and Add to Context (`.context/usability.md` "Add to Context").
 */

import { fileLinesUri, fileUri } from "./uri/uri.js";

/** The URI of a selection's lines (the same ones Ask About This names). */
export function selectionUri(
  path: string,
  sel: { startLineNumber: number; endLineNumber: number; endColumn: number },
  rev: string | null = null,
): string {
  const end = sel.endColumn === 1 && sel.endLineNumber > sel.startLineNumber ? sel.endLineNumber - 1 : sel.endLineNumber;
  return fileLinesUri(path, sel.startLineNumber, end, rev);
}

/** The editor's Add to Context entry: the selection's lines as text when
 *  there is one, else the whole file by its URI (its name as the label). */
export function editorContextEntry(
  path: string,
  sel: { startLineNumber: number; endLineNumber: number; endColumn: number; isEmpty(): boolean } | null,
  text: string | null,
): { label: string; what: { uri: string; text?: string; label?: string } } {
  if (sel && !sel.isEmpty()) return { label: "Add Selection to Context", what: { uri: selectionUri(path, sel), text: text ?? "" } };
  return { label: "Add File to Context", what: { uri: fileUri(path), label: path.split("/").pop() ?? path } };
}
