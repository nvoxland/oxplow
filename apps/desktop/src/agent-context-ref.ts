/**
 * Discriminated union + formatter for "things the user can add to the
 * agent's context". Format outputs a one-line text snippet ending with
 * a trailing space so the user can keep typing around it before
 * pressing Enter.
 *
 * Files and wiki pages use Claude Code's `@<path>` mention convention
 * so the agent reads the file on the next prompt. tasks have no
 * file form and instead get a short bracketed reference; the agent can
 * resolve the title to a body via `oxplow__get_task` if it cares.
 */

import { fileLinesRef } from "./refs/ref.js";

export type ContextRef =
  | { kind: "file"; path: string }
  | { kind: "wiki"; slug: string }
  | { kind: "task"; itemId: string; title: string; status: string }
  | { kind: "lens"; lensId: string; params: Record<string, unknown> }
  /** Ask About This (P6.D1): any canonical ref; the agent reads it by kind. */
  | { kind: "ref"; ref: string };

/** Ask About This on a selection (the editor, a diff's right side): the
 *  selected lines of `path` at `rev` (null = the working tree). A
 *  selection ending at column 1 of a line doesn't include that line. */
export function askAboutSelection(
  path: string,
  sel: { startLineNumber: number; endLineNumber: number; endColumn: number },
  rev: string | null = null,
): string {
  const end = sel.endColumn === 1 && sel.endLineNumber > sel.startLineNumber ? sel.endLineNumber - 1 : sel.endLineNumber;
  return formatContextMention({ kind: "ref", ref: fileLinesRef(path, sel.startLineNumber, end, rev) });
}

export function formatContextMention(ref: ContextRef): string {
  if (ref.kind === "file") {
    return `@${ref.path} `;
  }
  if (ref.kind === "wiki") {
    return `@.oxplow/wiki/${ref.slug}.md `;
  }
  if (ref.kind === "ref") {
    return `[oxplow ref ${ref.ref}] `;
  }
  if (ref.kind === "lens") {
    // `[oxplow lens <id> k=v …]` — the agent reads it with `get_lens` /
    // `run_lens`. Params sorted so the mention is stable.
    const params = Object.keys(ref.params)
      .sort()
      .map((k) => ` ${k}=${JSON.stringify(ref.params[k])}`)
      .join("");
    return `[oxplow lens ${ref.lensId}${params}] `;
  }
  // tasks: keep the title as plain text but strip newlines and
  // collapse internal whitespace so the inserted snippet stays on one
  // line. Don't escape quotes — the agent reads it as plain text.
  const cleanTitle = ref.title.replace(/\s+/g, " ").trim();
  return `[oxplow task ${ref.itemId}: "${cleanTitle}" (${ref.status})] `;
}
