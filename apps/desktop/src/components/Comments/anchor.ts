//! Resilient re-anchoring shared across comment surfaces.
//!
//! A comment's `quote` (the originally-selected text) is the durable
//! anchor; the stored position is only a hint. On load each surface
//! re-locates the quote in the *current* content and, if it can't,
//! marks the comment orphaned. The core search is surface-agnostic —
//! it works on a plain string — so rich-text (ProseMirror) and code
//! (Monaco) integrations both reuse it after flattening their content
//! to text.

/// Locate `quote` in `text`, preferring the occurrence whose start is
/// closest to `hintOffset` when there are several. Returns the start
/// offset, or `null` when the quote no longer appears (→ orphaned).
export function resolveQuoteOffset(
  text: string,
  quote: string,
  hintOffset?: number,
): number | null {
  if (quote.length === 0) return null;

  // Collect every occurrence. Quotes are usually short and unique, so
  // this is cheap; bail early on the common single-match case.
  const offsets: number[] = [];
  let from = 0;
  for (;;) {
    const idx = text.indexOf(quote, from);
    if (idx === -1) break;
    offsets.push(idx);
    from = idx + 1;
  }

  if (offsets.length === 0) return null;
  if (offsets.length === 1 || hintOffset === undefined) return offsets[0];

  // Disambiguate by proximity to the hint — content edits move the
  // quote, but rarely far, so the nearest match is the right one.
  let best = offsets[0];
  let bestDist = Math.abs(best - hintOffset);
  for (const off of offsets.slice(1)) {
    const dist = Math.abs(off - hintOffset);
    if (dist < bestDist) {
      best = off;
      bestDist = dist;
    }
  }
  return best;
}

// ---------------------------------------------------------------------
// Robust anchoring (W3C/Hypothesis-style): a quote selector enriched
// with surrounding context + a position hint, resolved in tiers with a
// bounded fuzzy fallback. `resolveQuoteOffset` above is the exact-only
// lookup (DOM annotations use it); the editors use `resolveAnchor` below.
// ---------------------------------------------------------------------

/// Chars of surrounding text captured/searched on each side of a quote.
export const CONTEXT_LEN = 32;
/// Reject a fuzzy candidate once more than this fraction of the needle
/// (prefix+quote+suffix) would have to change.
export const MAX_FUZZ = 0.25;
/// Half-width of the fuzzy search window around the position hint.
export const FUZZY_WINDOW = 64;
/// Below this length a quote only fuzzy-matches when it carries context
/// (a bare 1–3 char quote would match almost anything).
export const MIN_FUZZY_QUOTE_LEN = 4;
/// When there's no position hint, skip fuzzy on texts larger than this
/// (an unbounded O(needle·text) scan isn't worth it).
export const NO_HINT_FUZZY_LIMIT = 20_000;

export interface AnchorInput {
  quote: string;
  /// Up to CONTEXT_LEN chars immediately before the quote (may be "").
  prefix?: string;
  /// Up to CONTEXT_LEN chars immediately after the quote (may be "").
  suffix?: string;
  /// Selection-start char offset in the searched text — proximity hint.
  hintOffset?: number;
}

export interface ResolveResult {
  /// Resolved start offset, or null when the quote is lost (→ orphaned).
  offset: number | null;
  length: number;
  /// `exact` = literal quote found; `fuzzy` = approximate (drifted)
  /// match accepted within MAX_FUZZ; `none` = orphaned.
  confidence: "exact" | "fuzzy" | "none";
}

/// Pull the prefix/suffix context around `[start, end)` out of `text`.
/// Surfaces call this on the SAME flattened text the resolver searches
/// so stored context matches search context exactly.
export function extractContext(
  text: string,
  start: number,
  end: number,
): { prefix: string; suffix: string } {
  return {
    prefix: text.slice(Math.max(0, start - CONTEXT_LEN), start),
    suffix: text.slice(end, Math.min(text.length, end + CONTEXT_LEN)),
  };
}

function allOffsets(text: string, quote: string): number[] {
  const offsets: number[] = [];
  let from = 0;
  for (;;) {
    const idx = text.indexOf(quote, from);
    if (idx === -1) break;
    offsets.push(idx);
    from = idx + 1;
  }
  return offsets;
}

/// Count of trailing-prefix + leading-suffix chars that agree with the
/// neighbourhood of an occurrence at `off` — how well the surroundings
/// match. Used to disambiguate the same quote appearing many times.
function contextScore(
  text: string,
  off: number,
  qlen: number,
  prefix: string | undefined,
  suffix: string | undefined,
): number {
  let score = 0;
  if (prefix) {
    let i = prefix.length - 1;
    let j = off - 1;
    while (i >= 0 && j >= 0 && prefix[i] === text[j]) {
      score++;
      i--;
      j--;
    }
  }
  if (suffix) {
    let i = 0;
    let j = off + qlen;
    while (i < suffix.length && j < text.length && suffix[i] === text[j]) {
      score++;
      i++;
      j++;
    }
  }
  return score;
}

function pickBest(text: string, offsets: number[], a: AnchorInput): number {
  let best = offsets[0];
  let bestScore = -1;
  let bestDist = Infinity;
  for (const off of offsets) {
    const score = contextScore(text, off, a.quote.length, a.prefix, a.suffix);
    const dist = a.hintOffset === undefined ? 0 : Math.abs(off - a.hintOffset);
    if (score > bestScore || (score === bestScore && dist < bestDist)) {
      best = off;
      bestScore = score;
      bestDist = dist;
    }
  }
  return best;
}

/// Best approximate match of `pattern` anywhere in `hay` within
/// `maxDist` edits (Sellers' algorithm: row 0 is all-zero so a match may
/// start at any column; the answer is the min over the last row). Tracks
/// the start column so callers get the matched span, not just the end.
export function fuzzySubstring(
  pattern: string,
  hay: string,
  maxDist: number,
): { start: number; end: number; dist: number } | null {
  const m = pattern.length;
  const n = hay.length;
  if (m === 0 || n === 0) return null;

  let prevCost = new Array<number>(n + 1);
  let prevStart = new Array<number>(n + 1);
  let curCost = new Array<number>(n + 1);
  let curStart = new Array<number>(n + 1);
  for (let j = 0; j <= n; j++) {
    prevCost[j] = 0; // empty pattern matches at any column, free
    prevStart[j] = j;
  }
  for (let i = 1; i <= m; i++) {
    curCost[0] = i;
    curStart[0] = 0;
    for (let j = 1; j <= n; j++) {
      const sub = prevCost[j - 1] + (pattern[i - 1] === hay[j - 1] ? 0 : 1);
      const del = prevCost[j] + 1;
      const ins = curCost[j - 1] + 1;
      if (sub <= del && sub <= ins) {
        curCost[j] = sub;
        curStart[j] = prevStart[j - 1];
      } else if (del <= ins) {
        curCost[j] = del;
        curStart[j] = prevStart[j];
      } else {
        curCost[j] = ins;
        curStart[j] = curStart[j - 1];
      }
    }
    [prevCost, curCost] = [curCost, prevCost];
    [prevStart, curStart] = [curStart, prevStart];
  }

  let bestEnd = -1;
  let bestDist = maxDist + 1;
  for (let j = 1; j <= n; j++) {
    if (prevCost[j] < bestDist) {
      bestDist = prevCost[j];
      bestEnd = j;
    }
  }
  if (bestEnd === -1 || bestDist > maxDist) return null;
  return { start: prevStart[bestEnd], end: bestEnd, dist: bestDist };
}

function fuzzyResolve(text: string, a: AnchorInput): { offset: number; length: number } | null {
  const { quote, prefix = "", suffix = "", hintOffset } = a;
  const hasContext = prefix.length > 0 || suffix.length > 0;
  if (quote.length < MIN_FUZZY_QUOTE_LEN && !hasContext) return null;

  const needle = prefix + quote + suffix;
  const maxDist = Math.floor(needle.length * MAX_FUZZ);
  if (maxDist < 1) return null; // too short to fuzzy-match safely

  let lo = 0;
  let hi = text.length;
  if (hintOffset !== undefined) {
    const w = Math.max(quote.length, FUZZY_WINDOW);
    lo = Math.max(0, hintOffset - prefix.length - w);
    hi = Math.min(text.length, hintOffset - prefix.length + needle.length + w);
  } else if (text.length > NO_HINT_FUZZY_LIMIT) {
    return null;
  }

  const m = fuzzySubstring(needle, text.slice(lo, hi), maxDist);
  if (!m) return null;
  const matchStart = lo + m.start;
  const matchEnd = lo + m.end;
  // The quote sits after the prefix within the matched needle; clamp it
  // inside the matched region (fuzzy, so approximate by design).
  const qStart = Math.min(matchStart + prefix.length, matchEnd);
  const qLen = Math.min(quote.length, matchEnd - qStart);
  if (qLen <= 0) return null;
  return { offset: qStart, length: qLen };
}

/// Tiered resolve: exact quote (disambiguated by context + proximity),
/// then a bounded fuzzy fallback near the position hint. Returns the
/// offset/length plus a confidence the caller maps to orphaned/approx.
export function resolveAnchor(text: string, a: AnchorInput): ResolveResult {
  const length = a.quote.length;
  if (length === 0) return { offset: null, length: 0, confidence: "none" };

  const offsets = allOffsets(text, a.quote);
  if (offsets.length === 1) return { offset: offsets[0], length, confidence: "exact" };
  if (offsets.length > 1) {
    return { offset: pickBest(text, offsets, a), length, confidence: "exact" };
  }

  const fuzzy = fuzzyResolve(text, a);
  if (fuzzy) return { offset: fuzzy.offset, length: fuzzy.length, confidence: "fuzzy" };

  return { offset: null, length, confidence: "none" };
}

/// A resolved text-offset anchor. `null` offset means orphaned.
export interface ResolvedAnchor {
  offset: number | null;
  length: number;
}

/// Re-resolve a quote against current text, returning the new offset
/// (or orphaned) and the quote length. Surfaces convert `offset` into
/// their own coordinate space (ProseMirror positions, Monaco ranges).
export function reanchor(
  text: string,
  quote: string,
  hintOffset?: number,
): ResolvedAnchor {
  return { offset: resolveQuoteOffset(text, quote, hintOffset), length: quote.length };
}
