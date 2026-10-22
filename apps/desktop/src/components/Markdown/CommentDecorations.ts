import { Extension } from "@tiptap/react";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import type { Node as PMNode } from "@tiptap/pm/model";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

import { resolveAnchor } from "../Comments/anchor.js";

/// Separator inserted between blocks when flattening the doc to text, so
/// a selection (or quote search) can span block boundaries. Capture and
/// re-anchor both go through `flatten`, so the stored quote/context and
/// the searched text agree.
export const BLOCK_SEP = "\n";

/// A comment's resolved span in ProseMirror coordinates. `approx` marks
/// a fuzzy (drifted) re-attachment.
export interface CommentRange {
  id: string;
  from: number;
  to: number;
  approx?: boolean;
}

export const commentDecorationsKey = new PluginKey<DecorationSet>("commentDecorations");

/// ProseMirror plugin that paints inline highlights for the supplied
/// comment ranges. Critically these are **decorations, not stored
/// marks** — they overlay the document without touching the markdown
/// that round-trips to disk. Ranges are pushed in from React via a
/// transaction meta (`commentDecorationsKey`); between pushes the set
/// maps through doc edits so highlights track typing.
export const CommentDecorations = Extension.create<{
  onClickComment: ((id: string, rect: DOMRect) => void) | null;
}>({
  name: "commentDecorations",

  addOptions() {
    return { onClickComment: null };
  },

  addProseMirrorPlugins() {
    const options = this.options;
    return [
      new Plugin<DecorationSet>({
        key: commentDecorationsKey,
        state: {
          init() {
            return DecorationSet.empty;
          },
          apply(tr, old) {
            const next = tr.getMeta(commentDecorationsKey) as CommentRange[] | undefined;
            if (next) {
              const decos = next
                .filter((r) => r.from < r.to)
                .map((r) =>
                  Decoration.inline(r.from, r.to, {
                    class: r.approx
                      ? "oxplow-comment-highlight oxplow-comment-highlight--approx"
                      : "oxplow-comment-highlight",
                    "data-comment-id": String(r.id),
                  }),
                );
              return DecorationSet.create(tr.doc, decos);
            }
            return old.map(tr.mapping, tr.doc);
          },
        },
        props: {
          decorations(state) {
            return commentDecorationsKey.getState(state);
          },
          handleClick(_view, _pos, event) {
            const target = event.target as HTMLElement | null;
            const el = target?.closest?.("[data-comment-id]") as HTMLElement | null;
            if (el && options.onClickComment) {
              options.onClickComment(el.getAttribute("data-comment-id") ?? "", el.getBoundingClientRect());
              return true;
            }
            return false;
          },
        },
      }),
    ];
  },
});

/// Flatten a doc's text nodes into a single string plus an offset→pos
/// map, so a plain-text quote search can be mapped back to ProseMirror
/// document positions. A `BLOCK_SEP` char is inserted between adjacent
/// text runs in different blocks (with a synthetic map entry, so `map`
/// stays 1:1 with `text`), letting quotes span block boundaries.
export function flatten(doc: PMNode): { text: string; map: number[] } {
  let text = "";
  const map: number[] = []; // map[textOffset] = doc pos of that char
  let separated = true; // suppress a leading separator
  doc.descendants((node, pos) => {
    if (node.isBlock) {
      if (text.length > 0 && !separated) {
        text += BLOCK_SEP;
        map.push(pos); // separator maps to the block's start pos
        separated = true;
      }
      return true;
    }
    if (node.isText && node.text) {
      for (let i = 0; i < node.text.length; i++) {
        map.push(pos + i);
      }
      text += node.text;
      separated = false;
    }
    return true;
  });
  return { text, map };
}

/// Re-resolve a quote to a `{ from, to, approx }` doc range via the
/// shared tiered resolver: stored hint range fast path, then exact
/// (context/proximity disambiguated), then bounded fuzzy. Returns `null`
/// when the quote is lost → orphaned. `approx` is true for a fuzzy match.
export function findCommentRange(
  doc: PMNode,
  quote: string,
  opts: { hintFrom?: number; hintTo?: number; prefix?: string; suffix?: string } = {},
): { from: number; to: number; approx: boolean } | null {
  if (quote.length === 0) return null;
  const { hintFrom, hintTo, prefix, suffix } = opts;

  // Fast path: the hint range still spells the quote exactly.
  if (
    hintFrom !== undefined &&
    hintTo !== undefined &&
    hintFrom >= 0 &&
    hintTo <= doc.content.size &&
    hintFrom < hintTo
  ) {
    if (doc.textBetween(hintFrom, hintTo, BLOCK_SEP) === quote) {
      return { from: hintFrom, to: hintTo, approx: false };
    }
  }

  const { text, map } = flatten(doc);
  // Translate the hint's doc pos into a text offset for proximity.
  const hintIdx = hintFrom !== undefined ? map.findIndex((p) => p >= hintFrom) : -1;
  const res = resolveAnchor(text, {
    quote,
    prefix,
    suffix,
    hintOffset: hintIdx === -1 ? undefined : hintIdx,
  });
  if (res.offset === null) return null;
  const startPos = map[res.offset];
  const endPos = map[res.offset + res.length - 1];
  if (startPos === undefined || endPos === undefined) return null;
  return { from: startPos, to: endPos + 1, approx: res.confidence === "fuzzy" };
}
