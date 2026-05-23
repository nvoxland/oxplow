//! Anchoring helpers for comments on xterm.js terminal output.
//!
//! The terminal has no DOM model of its full scrollback — only the
//! visible viewport lives in the DOM. So we anchor against the
//! *serialized buffer text*: flatten every buffer line into one string,
//! resolve the durable quote into a char offset there (via the shared
//! `resolveAnchor`), then map that offset back to a `(line, col)` buffer
//! coordinate so an xterm decoration can be placed on it.
//!
//! Scrollback is ephemeral and wraps/evicts, so terminal anchors orphan
//! more readily than on a stable document — that's expected and handled
//! by the existing orphan path (no decoration painted, comment still
//! listed in the inbox).

import { resolveAnchor } from "./anchor.js";
import {
  anchorInputFromSelectors,
  buildTextSelectors,
  type WebSelector,
} from "./selectors.js";

/// A flattened view of the terminal buffer: every line joined by `\n`,
/// plus the char offset at which each buffer line begins (so an offset
/// in `text` maps back to a `(line, col)` coordinate).
export interface SerializedBuffer {
  text: string;
  lineStarts: number[];
}

/// Flatten buffer lines (each already `translateToString()`-ed) into a
/// [`SerializedBuffer`]. `lineStarts[i]` is the char offset of line `i`
/// in `text`; lines are joined with a single `\n`.
export function serializeBuffer(lines: string[]): SerializedBuffer {
  const lineStarts: number[] = [];
  let offset = 0;
  for (const line of lines) {
    lineStarts.push(offset);
    offset += line.length + 1; // +1 for the joining newline
  }
  return { text: lines.join("\n"), lineStarts };
}

/// Map a char offset in the serialized text back to a buffer
/// `(line, col)`. Clamps to the last line for an out-of-range offset.
export function offsetToCoord(buf: SerializedBuffer, offset: number): { line: number; col: number } {
  const starts = buf.lineStarts;
  if (starts.length === 0) return { line: 0, col: 0 };
  // Binary search for the greatest lineStart <= offset.
  let lo = 0;
  let hi = starts.length - 1;
  let line = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (starts[mid]! <= offset) {
      line = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return { line, col: offset - starts[line]! };
}

/// Map a buffer `(line, col)` to a char offset in the serialized text.
export function coordToOffset(buf: SerializedBuffer, line: number, col: number): number {
  const base = buf.lineStarts[line] ?? buf.text.length;
  return base + col;
}

/// xterm buffer-coordinate selector — the per-surface coordinate
/// refinement stored alongside the W3C text selectors (the terminal
/// analog of Monaco's range selector / ProseMirror's position selector).
export interface TerminalBufferSelector {
  type: "TerminalBufferSelector";
  startLine: number;
  startCol: number;
  endLine: number;
  endCol: number;
}

/// Build `selectors_json` for a terminal selection spanning the char
/// range `[start, end)` of the serialized buffer text: the standard W3C
/// text selectors plus the buffer-coordinate refinement.
export function buildTerminalSelectorsJson(
  buf: SerializedBuffer,
  start: number,
  end: number,
): string {
  const a = offsetToCoord(buf, start);
  const b = offsetToCoord(buf, end);
  const selectors: (WebSelector | TerminalBufferSelector)[] = [
    ...buildTextSelectors(buf.text, start, end),
    {
      type: "TerminalBufferSelector",
      startLine: a.line,
      startCol: a.col,
      endLine: b.line,
      endCol: b.col,
    },
  ];
  return JSON.stringify(selectors);
}

/// The buffer-coordinate span a comment re-anchors to in the current
/// buffer, plus the resolver's confidence (`fuzzy` → dashed/approx).
export interface TerminalAnchor {
  startLine: number;
  startCol: number;
  endLine: number;
  endCol: number;
  confidence: "exact" | "fuzzy";
}

/// Re-locate a comment's `quote` in the current buffer text and return
/// the buffer-coordinate span to decorate, or `null` when it can't be
/// found (the comment orphans for this surface). The quote is the durable
/// fact; the stored selectors only hint position/context.
export function reanchorInBuffer(
  buf: SerializedBuffer,
  selectorsJson: string,
  quote: string,
): TerminalAnchor | null {
  const res = resolveAnchor(buf.text, anchorInputFromSelectors(selectorsJson, quote));
  if (res.offset === null) return null;
  const a = offsetToCoord(buf, res.offset);
  const b = offsetToCoord(buf, res.offset + res.length);
  return {
    startLine: a.line,
    startCol: a.col,
    endLine: b.line,
    endCol: b.col,
    confidence: res.confidence === "fuzzy" ? "fuzzy" : "exact",
  };
}
