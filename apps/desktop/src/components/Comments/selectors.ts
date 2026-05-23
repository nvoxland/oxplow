//! Formal W3C-Web-Annotation selector model for comment anchors.
//!
//! A comment's durable anchor is its `quote`; `selectors_json` carries a
//! richer, standard hint so a quote can be re-located fast and
//! disambiguated. We store an array of selectors following the
//! [W3C Web Annotation Data Model](https://www.w3.org/TR/annotation-model/):
//!
//!   - a `TextQuoteSelector` — the exact text plus up to CONTEXT_LEN
//!     chars of `prefix`/`suffix` (Hypothesis's robust-anchoring shape);
//!   - a `TextPositionSelector` — the `[start, end)` char offsets in the
//!     anchoring element's flattened `textContent`, a proximity hint.
//!
//! Editor surfaces (Monaco, ProseMirror) historically stored a single
//! position *object* instead; [`anchorInputFromSelectors`] reads either
//! shape so every surface shares one resolver ([`resolveAnchor`]).

import { CONTEXT_LEN, type AnchorInput, extractContext } from "./anchor.js";

/// W3C TextQuoteSelector — the durable, content-based selector.
export interface TextQuoteSelector {
  type: "TextQuoteSelector";
  exact: string;
  prefix?: string;
  suffix?: string;
}

/// W3C TextPositionSelector — character offsets into the normalized text
/// of the anchoring resource. A hint, re-validated on load.
export interface TextPositionSelector {
  type: "TextPositionSelector";
  start: number;
  end: number;
}

export type WebSelector = TextQuoteSelector | TextPositionSelector;

/// Build the W3C selectors array for the span `[start, end)` of `text`
/// (the anchoring element's flattened `textContent`). `text.slice` gives
/// the exact quote; `extractContext` pulls the surrounding prefix/suffix
/// from the SAME text the resolver will search, so they line up.
export function buildTextSelectors(text: string, start: number, end: number): WebSelector[] {
  const exact = text.slice(start, end);
  const { prefix, suffix } = extractContext(text, start, end);
  return [
    { type: "TextQuoteSelector", exact, prefix, suffix },
    { type: "TextPositionSelector", start, end },
  ];
}

/// Serialize the selectors for the span into the `selectors_json` column.
export function buildSelectorsJson(text: string, start: number, end: number): string {
  return JSON.stringify(buildTextSelectors(text, start, end));
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

/// Read an [`AnchorInput`] (for [`resolveAnchor`]) out of a stored
/// `selectors_json` + the comment's durable `quote`. Tolerant of two
/// shapes:
///
///   - the W3C array this module writes (TextQuote + TextPosition);
///   - the legacy per-surface position *object* the Monaco / ProseMirror
///     layers write (`{ prefix, suffix, textOffset }`).
///
/// `quote` always wins over a selector's `exact` — it is the durable
/// fact; the selectors are only hints. Missing/garbage JSON degrades to
/// a bare quote anchor (still resolvable, just without context/proximity).
export function anchorInputFromSelectors(selectorsJson: string, quote: string): AnchorInput {
  let prefix: string | undefined;
  let suffix: string | undefined;
  let hintOffset: number | undefined;

  try {
    const parsed: unknown = JSON.parse(selectorsJson);
    if (Array.isArray(parsed)) {
      for (const sel of parsed) {
        if (!isRecord(sel)) continue;
        if (sel.type === "TextQuoteSelector") {
          if (typeof sel.prefix === "string") prefix = sel.prefix;
          if (typeof sel.suffix === "string") suffix = sel.suffix;
        } else if (sel.type === "TextPositionSelector") {
          if (typeof sel.start === "number") hintOffset = sel.start;
        }
      }
    } else if (isRecord(parsed)) {
      // Legacy position-object shape (Monaco / ProseMirror).
      if (typeof parsed.prefix === "string") prefix = parsed.prefix;
      if (typeof parsed.suffix === "string") suffix = parsed.suffix;
      if (typeof parsed.textOffset === "number") hintOffset = parsed.textOffset;
    }
  } catch {
    // fall through to a bare quote anchor
  }

  // Clamp stored context to the resolver's window so a longer legacy
  // prefix/suffix can't skew the fuzzy needle.
  if (prefix && prefix.length > CONTEXT_LEN) prefix = prefix.slice(-CONTEXT_LEN);
  if (suffix && suffix.length > CONTEXT_LEN) suffix = suffix.slice(0, CONTEXT_LEN);

  return { quote, prefix, suffix, hintOffset };
}
