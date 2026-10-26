/**
 * Discriminated union + formatter for "things the user can add to the
 * agent's context". Format outputs a one-line text snippet ending with
 * a trailing space so the user can keep typing around it before
 * pressing Enter.
 *
 * Files and wiki pages use Claude Code's `@<path>` mention convention
 * so the agent reads the file on the next prompt. Work items have no
 * file form and instead get a short bracketed reference by ref; the agent
 * reads the item through the work-item interface if it cares.
 */

import { fileLinesUri } from "./uri/uri.js";

export type ContextRef =
  | { kind: "file"; path: string }
  | { kind: "wiki"; slug: string }
  | { kind: "work_item"; ref: string; title: string; state: string }
  /** Ask About This (P6.D1): any canonical ref; the agent reads it by kind. */
  | { kind: "ref"; ref: string };

/** The URI of a selection's lines (the same ones Ask About This names). */
export function selectionUri(
  path: string,
  sel: { startLineNumber: number; endLineNumber: number; endColumn: number },
  rev: string | null = null,
): string {
  const end = sel.endColumn === 1 && sel.endLineNumber > sel.startLineNumber ? sel.endLineNumber - 1 : sel.endLineNumber;
  return fileLinesUri(path, sel.startLineNumber, end, rev);
}

export function formatContextMention(ref: ContextRef): string {
  if (ref.kind === "file") {
    return `@${ref.path} `;
  }
  if (ref.kind === "wiki") {
    return `@.oxplow/wiki/${ref.slug}.md `;
  }
  if (ref.kind === "ref") {
    return `[oxplow uri ${ref.ref}] `;
  }
  // A work item: keep the title as plain text but strip newlines and
  // collapse internal whitespace so the inserted snippet stays on one
  // line. Don't escape quotes — the agent reads it as plain text.
  const cleanTitle = ref.title.replace(/\s+/g, " ").trim();
  return `[oxplow ${ref.ref}: "${cleanTitle}" (${ref.state})] `;
}
