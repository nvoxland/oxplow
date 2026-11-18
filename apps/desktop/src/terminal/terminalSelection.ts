/// A terminal's selection for the context basket. Its scrollback has no
/// address an agent can read back, so the selection goes with the lines
/// around it (`oxplow.context.add`'s `around`): a few rows either side, a
/// wrapped row joined to the row it continues, trailing spaces and blank
/// edges dropped.
import type { Terminal } from "@xterm/xterm";

/** One buffer row: its text, and whether it continues the row above. */
export interface Row {
  text: string;
  wrapped: boolean;
}

/** Rows `start`..`end` (0-based, inclusive) and `pad` rows either side, as
 *  lines. */
export function linesAround(row: (y: number) => Row | undefined, start: number, end: number, pad = 3): string {
  const lines: string[] = [];
  for (let y = Math.max(0, start - pad); y <= end + pad; y++) {
    const r = row(y);
    if (!r) break;
    // A wrapped row continues the line above as it is: its spaces are text.
    if (r.wrapped && lines.length > 0) lines[lines.length - 1] += r.text;
    else lines.push(r.text);
  }
  for (let i = 0; i < lines.length; i++) lines[i] = lines[i]!.trimEnd();
  while (lines.length > 0 && !lines[0]!.trim()) lines.shift();
  while (lines.length > 0 && !lines[lines.length - 1]!.trim()) lines.pop();
  return lines.join("\n");
}

/** What Add to Context puts in the basket for `term`'s selection, on the
 *  terminal's page `uri`; null when nothing is selected. */
export function terminalSelectionWhat(term: Terminal, uri: string): { uri: string; text: string; around?: string } | null {
  const text = term.getSelection();
  if (!text.trim()) return null;
  const at = term.getSelectionPosition();
  if (!at) return { uri, text };
  const buffer = term.buffer.active;
  // The position's rows are 1-based.
  const around = linesAround(
    (y) => {
      const line = buffer.getLine(y);
      return line ? { text: line.translateToString(true), wrapped: line.isWrapped } : undefined;
    },
    at.start.y - 1,
    at.end.y - 1,
  );
  return around && around !== text.trim() ? { uri, text, around } : { uri, text };
}
