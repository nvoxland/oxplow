//! Thin wrapper over the CSS Custom Highlight API for painting comment
//! highlights on plain DOM without mutating it.
//!
//! Unlike wrapping selected text in `<span>`s — which fights React
//! reconciliation, breaks row event handlers, and shifts layout — the
//! Custom Highlight API paints `::highlight(name)` over live `Range`s
//! the browser keeps in sync. Targets here are constantly-re-rendering
//! React lists (task rows, the commit graph), so non-mutating painting
//! is essential.
//!
//! The API is feature-detected: in a WebView that lacks it (or the bun
//! test env) every call is a no-op, so comments still function — they
//! just don't show an inline highlight, exactly like an orphaned one.
//! See `.context/editor-and-monaco.md` for the editor surfaces, which
//! keep their own decoration mechanisms.

/// Registry name for exact-match highlights and for drifted (fuzzy)
/// ones, themed separately in the global stylesheet via
/// `::highlight(oxplow-comment)` / `::highlight(oxplow-comment-approx)`.
const EXACT = "oxplow-comment";
const APPROX = "oxplow-comment-approx";

// Minimal shapes for the parts of the API we use (not yet in every
// TS lib.dom). Feature-detected before use, so the casts are sound.
interface HighlightCtor {
  new (...ranges: Range[]): unknown;
}
interface HighlightRegistry {
  set(name: string, highlight: unknown): void;
  delete(name: string): void;
}

function registry(): HighlightRegistry | null {
  const css = (globalThis as { CSS?: { highlights?: HighlightRegistry } }).CSS;
  return css?.highlights ?? null;
}

function highlightCtor(): HighlightCtor | null {
  return (globalThis as { Highlight?: HighlightCtor }).Highlight ?? null;
}

/// True when the running WebView supports the CSS Custom Highlight API.
export function highlightApiAvailable(): boolean {
  return registry() !== null && highlightCtor() !== null;
}

function setOrDelete(reg: HighlightRegistry, Ctor: HighlightCtor, name: string, ranges: Range[]) {
  if (ranges.length === 0) {
    reg.delete(name);
    return;
  }
  reg.set(name, new Ctor(...ranges));
}

/// Replace the registered comment highlights with the given ranges.
/// `exact` and `approx` are themed differently (approx = a drifted fuzzy
/// match, shown dashed). No-op when the API is unavailable.
export function paintCommentHighlights(exact: Range[], approx: Range[]): void {
  const reg = registry();
  const Ctor = highlightCtor();
  if (!reg || !Ctor) return;
  setOrDelete(reg, Ctor, EXACT, exact);
  setOrDelete(reg, Ctor, APPROX, approx);
}

/// Remove all comment highlights (e.g. on unmount).
export function clearCommentHighlights(): void {
  const reg = registry();
  if (!reg) return;
  reg.delete(EXACT);
  reg.delete(APPROX);
}
