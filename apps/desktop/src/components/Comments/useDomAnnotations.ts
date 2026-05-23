//! Capture a text selection on any plain-DOM surface into a comment
//! anchor + its typed context.
//!
//! The flow: on `mouseup` with a non-collapsed selection inside a
//! commentable region (not an editor/input/terminal — the same carve-out
//! as the native context menu), we snapshot the selection into a
//! [`PendingComment`]. A floating toolbar shows over it; "Add comment"
//! opens the composer; submitting calls `createComment` with the quote,
//! the W3C selectors, the primary target (nearest context node), the
//! context chain (ancestor regions), and any refs linked inside the
//! selection. See `.context/usability.md` (selection affordance) and
//! `contextNodes.tsx` (the `data-ref-*` hierarchy).

import { useCallback, useEffect, useRef, useState } from "react";

import { describeChain, shouldSuppressContextMenu } from "../../context-menu.js";
import { resolveQuoteOffset } from "./anchor.js";
import {
  collectContextChain,
  nearestContextElement,
  type RefNode,
  refOfElement,
} from "./contextNodes.js";
import { refsInRange } from "./domAnchor.js";
import { buildSelectorsJson } from "./selectors.js";

/// A snapshot of a selection, ready to become a comment. Captured at
/// selection time so it survives the user clicking the toolbar (which
/// would otherwise collapse the live selection).
export interface PendingComment {
  /// Primary anchor — the nearest context node `(kind,id)`.
  targetKind: string;
  targetId: string;
  /// The durable selected text.
  quote: string;
  /// W3C selectors array (TextQuote + TextPosition) over the primary
  /// element's `textContent`.
  selectorsJson: string;
  /// Ancestor regions the selection sat inside (excluding the primary).
  contextChain: RefNode[];
  /// Canonical refs linked inside the selection.
  referencedRefs: RefNode[];
  /// Viewport rect of the selection — where to anchor the toolbar/composer.
  rect: DOMRect;
}

/// Char offset of `(container, offset)` within `root`'s text, measured
/// the same way `textContent` concatenates it.
function offsetWithin(root: Element, container: Node, offset: number): number {
  const doc = root.ownerDocument;
  if (!doc) return 0;
  const r = doc.createRange();
  r.setStart(root, 0);
  try {
    r.setEnd(container, offset);
  } catch {
    return 0;
  }
  return r.toString().length;
}

/// Snapshot the current window selection into a [`PendingComment`], or
/// `null` when there's nothing commentable (collapsed, whitespace-only,
/// inside an editor/input, or outside any context node).
export function captureSelection(): PendingComment | null {
  const sel = window.getSelection();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return null;
  const raw = sel.toString();
  const quote = raw.trim();
  if (!quote) return null;

  const range = sel.getRangeAt(0);
  // Same carve-out as the native context menu: skip editors, inputs,
  // contenteditable, and the terminal — they own their own selection UX.
  if (!shouldSuppressContextMenu(describeChain(range.startContainer))) return null;

  const primaryEl = nearestContextElement(range.startContainer);
  if (!primaryEl) return null;
  const primary = refOfElement(primaryEl);
  if (!primary) return null;

  // Ancestors above the primary form the context chain.
  const contextChain = collectContextChain(primaryEl.parentElement);

  // Resolve the quote to a char offset within the primary element's
  // textContent so the stored selectors line up with what the renderer
  // re-resolves on load.
  const text = primaryEl.textContent ?? "";
  const lead = raw.length - raw.trimStart().length;
  const hint = offsetWithin(primaryEl, range.startContainer, range.startOffset) + lead;
  const start = resolveQuoteOffset(text, quote, hint) ?? text.indexOf(quote);
  if (start < 0) return null; // quote not in the element's text — bail
  const selectorsJson = buildSelectorsJson(text, start, start + quote.length);

  return {
    targetKind: primary.kind,
    targetId: primary.id,
    quote,
    selectorsJson,
    contextChain,
    referencedRefs: refsInRange(range),
    rect: range.getBoundingClientRect(),
  };
}

export interface DomAnnotations {
  /// The captured selection awaiting a toolbar click, or `null`.
  pending: PendingComment | null;
  /// Whether the composer (vs the toolbar) is showing.
  composing: boolean;
  /// Toolbar "Add comment" → open the composer for `pending`.
  beginCompose: () => void;
  /// Dismiss the toolbar/composer and forget the pending selection.
  cancel: () => void;
}

/// Listen for selections on the document and expose the capture/compose
/// state machine. Mount once per commentable page (see `DomCommentLayer`).
export function useDomAnnotations(): DomAnnotations {
  const [pending, setPending] = useState<PendingComment | null>(null);
  const [composing, setComposing] = useState(false);
  // Keep the listener stable while reading the latest composing flag.
  const composingRef = useRef(composing);
  composingRef.current = composing;

  useEffect(() => {
    const onMouseUp = (e: MouseEvent) => {
      // Ignore clicks inside our own toolbar/composer (marked with the
      // attribute below) and don't disturb an open composer.
      if (composingRef.current) return;
      if ((e.target as Element | null)?.closest?.("[data-selection-comment-ui]")) return;
      // Defer to let the browser finalize the selection after mouseup.
      window.setTimeout(() => {
        if (composingRef.current) return;
        setPending(captureSelection());
      }, 0);
    };
    document.addEventListener("mouseup", onMouseUp);
    return () => document.removeEventListener("mouseup", onMouseUp);
  }, []);

  const beginCompose = useCallback(() => setComposing(true), []);
  const cancel = useCallback(() => {
    setComposing(false);
    setPending(null);
  }, []);

  return { pending, composing, beginCompose, cancel };
}
