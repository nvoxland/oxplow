//! DOM helpers for anchoring comments to plain (non-editor) page text.
//!
//! Editor surfaces map a quote into their own coordinate space (Monaco
//! ranges, ProseMirror positions). For everything else — task rows, the
//! file list, the git dashboard — the anchoring element's flattened
//! `textContent` IS the coordinate space: we resolve the quote to a
//! `[start, length)` char span (via the shared `resolveAnchor`) and turn
//! that span back into a DOM `Range` to paint, without mutating the DOM.

import { commitUri, dirUri, fileUri, wikiUri } from "../../uri/uri.js";
import { parseMarkdownLink } from "../Wiki/MarkdownView.js";

/// Convert a character span of `root.textContent` into a DOM `Range` by
/// walking the element's text nodes in document order (the same order
/// `textContent` concatenates them). Returns `null` when the span falls
/// outside the current text — the caller treats that as "can't paint"
/// (the comment is shown as orphaned for this surface).
export function textContentRange(root: Element, start: number, length: number): Range | null {
  if (start < 0 || length <= 0) return null;
  const end = start + length;
  const doc = root.ownerDocument;
  if (!doc) return null;
  const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT);

  let pos = 0;
  let startNode: Text | null = null;
  let startOffset = 0;
  let endNode: Text | null = null;
  let endOffset = 0;

  let node = walker.nextNode() as Text | null;
  while (node) {
    const len = node.data.length;
    if (startNode === null && start < pos + len) {
      startNode = node;
      startOffset = start - pos;
    }
    if (startNode !== null && end <= pos + len) {
      endNode = node;
      endOffset = end - pos;
      break;
    }
    pos += len;
    node = walker.nextNode() as Text | null;
  }

  if (!startNode || !endNode) return null;
  const range = doc.createRange();
  range.setStart(startNode, startOffset);
  range.setEnd(endNode, endOffset);
  return range;
}

/// Map a rendered link href to the URI it names, or `null` for hrefs that
/// don't point at a first-class page (external, anchors).
export function refFromHref(href: string): string | null {
  const parsed = parseMarkdownLink(href);
  switch (parsed.kind) {
    case "internal":
      return wikiUri(parsed.slug);
    case "file":
      return fileUri(parsed.path);
    case "directory":
      return dirUri(parsed.path);
    case "commit":
      return commitUri(parsed.sha);
    case "work_item":
      return parsed.ref;
    default:
      return null;
  }
}

/// The URIs pointed at by any `<a>` links inside `range` —
/// "text that is *about* a thing is a link to that thing", so a link in
/// the selection becomes typed context. Deduped, in document order.
/// Partially-selected links count (their typed target is still relevant).
export function refsInRange(range: Range): string[] {
  const refs: string[] = [];
  const seen = new Set<string>();
  const frag = range.cloneContents();
  for (const a of Array.from(frag.querySelectorAll("a[href]"))) {
    const ref = refFromHref(a.getAttribute("href") ?? "");
    if (!ref) continue;
    if (!seen.has(ref)) {
      seen.add(ref);
      refs.push(ref);
    }
  }
  return refs;
}
