/**
 * Path-detection + xterm.js link-provider wiring.
 *
 * `findFilePathMatches` is the pure, unit-tested path scanner; it
 * returns offset ranges plus optional `:line[:col]` for each match in
 * a line of terminal text. `installFilePathLinkProvider` registers
 * those ranges as xterm ILinks so they render underlined and route a
 * click through `onActivate`.
 */

import type { IDisposable, ILink, ILinkProvider, Terminal } from "@xterm/xterm";

export interface FilePathMatch {
  /** Inclusive start offset in the line text (0-based). */
  start: number;
  /** Exclusive end offset in the line text. */
  end: number;
  /** Path text without the `:line[:col]` suffix. */
  text: string;
  line: number | undefined;
  column: number | undefined;
}

// Token: a run of characters that could plausibly be part of a path,
// optionally followed by `:N` or `:N:M`. Whitespace, quote-like, and
// bracket-like chars are boundaries.
const TOKEN_RE = /[\w./@~+-]+(?::\d+(?::\d+)?)?/g;

const TRAILING_PUNCT = /[.,;!?)\]}>'"`]+$/;
const LEADING_PUNCT = /^['"`(\[<]+/;

// A scheme like `http://` or `file://` immediately preceding the
// match means this token is part of a URL — skip it. The trailing
// `\/?\/?` covers all three slice-ends because tokens may start at
// any of the slashes (`://example.com` token starts at `/`, `//x`
// token starts at the second `/`, etc.).
const URL_PREFIX_RE = /[a-zA-Z][\w+.-]*:\/?\/?$/;

/**
 * Find file-path-shaped substrings in `line`. `validatePath`, when given, is a
 * final gate: a candidate that passes the shape heuristics but `validatePath`
 * rejects is dropped, so dotted words in prose that aren't real workspace
 * targets (e.g. a plugin name like `oxplow.junit`) don't become links. It's
 * given the bare path (no `:line:col` suffix).
 *
 * NOTE: the memoized link provider deliberately calls this **without**
 * `validatePath` — it caches the shape-only matches (which are a pure function
 * of the text) and applies the live `validatePath` gate itself on the cached
 * result (see `installFilePathLinkProvider` / `createMemoizedScanner`). Do NOT
 * wire `validatePath` into the memoized scanner: the gate depends on the
 * workspace index, which changes, so baking it into the cache would serve stale
 * link decisions. The parameter stays for direct/one-shot callers and tests.
 */
export function findFilePathMatches(
  line: string,
  validatePath?: (path: string) => boolean,
): FilePathMatch[] {
  const out: FilePathMatch[] = [];
  TOKEN_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = TOKEN_RE.exec(line)) !== null) {
    const raw = m[0];
    const rawStart = m.index;
    const rawEnd = rawStart + raw.length;

    // Strip leading/trailing prose punctuation.
    let stem = raw;
    let start = rawStart;
    let end = rawEnd;
    const lead = LEADING_PUNCT.exec(stem);
    if (lead) {
      stem = stem.slice(lead[0].length);
      start += lead[0].length;
    }
    const trail = TRAILING_PUNCT.exec(stem);
    if (trail) {
      stem = stem.slice(0, stem.length - trail[0].length);
      end -= trail[0].length;
    }
    if (!stem) continue;

    // Skip if preceded by a URL scheme (`http://`, `file://`, etc).
    const before = line.slice(0, start);
    if (URL_PREFIX_RE.test(before)) continue;

    // Email-shaped (`name@host.tld`) — token contains `@` followed by
    // a dotted host. Reject.
    if (/^[\w.+-]+@[\w.-]+\.\w+$/.test(stem)) continue;

    // Pull off optional `:line[:col]` suffix.
    let pathText = stem;
    let lineNum: number | undefined;
    let colNum: number | undefined;
    const lc = /^(.*?)(?::(\d+)(?::(\d+))?)$/.exec(stem);
    if (lc) {
      pathText = lc[1];
      lineNum = Number(lc[2]);
      if (lc[3] !== undefined) colNum = Number(lc[3]);
    }
    if (!pathText) continue;

    // Validation: must look like a path. Either contain a `/` or have
    // a recognizable filename.ext shape.
    if (!looksLikePath(pathText)) continue;
    // Final gate: drop shapes that aren't real workspace targets.
    if (validatePath && !validatePath(pathText)) continue;

    out.push({
      start,
      end,
      text: pathText,
      line: lineNum,
      column: colNum,
    });
  }
  return out;
}

function looksLikePath(s: string): boolean {
  // Pure number (`1.5`, `42`, `3.14`) — reject.
  if (/^\d+(?:\.\d+)*$/.test(s)) return false;

  if (s.includes("/")) {
    // Must have at least one alphabetic segment somewhere — `1/2` and
    // `1/2/3` are arithmetic, not paths. The simplest test that covers
    // the realistic cases: at least one letter anywhere.
    return /[a-zA-Z]/.test(s);
  }

  // No slash — accept only if it looks like `name.ext` with a real
  // extension. Extension must start with a letter and be 1–8 chars.
  // Rules out version strings (`1.5`), domain-y bits, etc.
  const dot = /^([\w.@~+-]+)\.([a-zA-Z][a-zA-Z0-9]{0,7})$/.exec(s);
  if (!dot) return false;
  // Stem must contain a letter (rules out `1.5` even if extensions
  // were lenient).
  if (!/[a-zA-Z]/.test(dot[1])) return false;
  return true;
}

/** Max distinct line-texts kept in the per-provider scan memo. xterm only
 *  asks `provideLinks` about visible/hovered rows, so the working set is
 *  tiny (a viewport plus whatever's been hovered); this cap just bounds a
 *  long streaming session's growth. */
const MATCH_CACHE_MAX = 512;

/**
 * Wrap a line scanner in a bounded, text-keyed memo. xterm re-invokes
 * `provideLinks` for every visible/hovered row on each scroll and hover,
 * so re-scanning unchanged rows is pure per-render waste. Keying the
 * cache by the row's text makes a re-hover / re-scroll over unchanged
 * content a hit, and auto-invalidates when a row's content changes (new
 * text ⇒ new key). Bounded, true **LRU**: a hit refreshes recency (Map
 * insertion order = access order), so a frequently re-hovered stable row
 * isn't evicted just for being inserted early; eviction drops the
 * least-recently-used key.
 *
 * The scanner passed here must be the *shape-only* pass (no
 * `validatePath` gate): that gate can change as the workspace index
 * updates, so it's applied fresh by the caller on the cached shape
 * matches, never baked into the cache.
 */
export function createMemoizedScanner(
  scan: (text: string) => FilePathMatch[],
  max = MATCH_CACHE_MAX,
): (text: string) => FilePathMatch[] {
  const cache = new Map<string, FilePathMatch[]>();
  return (text: string): FilePathMatch[] => {
    const cached = cache.get(text);
    if (cached) {
      // Refresh recency: re-insert so this key moves to the newest slot.
      cache.delete(text);
      cache.set(text, cached);
      return cached;
    }
    const matches = scan(text);
    cache.set(text, matches);
    if (cache.size > max) {
      const oldest = cache.keys().next().value; // least-recently-used
      if (oldest !== undefined) cache.delete(oldest);
    }
    return matches;
  };
}

export interface FilePathLinkActivation {
  /** Path text as it appeared in the terminal (no line/col suffix). */
  text: string;
  line: number | undefined;
  column: number | undefined;
}

/**
 * Wire the file-path scanner into an xterm Terminal as a link
 * provider. `onActivate` is called when the user clicks a detected
 * link; `getHover` (optional) lets the host customize the hover
 * tooltip — defaults to "Open <text>".
 *
 * Returns the IDisposable from `term.registerLinkProvider` so the
 * caller can dispose it on terminal teardown.
 */
export function installFilePathLinkProvider(
  term: Terminal,
  opts: {
    onActivate(match: FilePathLinkActivation): void;
    getHover?(match: FilePathLinkActivation): string;
    /** Final gate: only linkify a path this returns true for. Read fresh on
     * every line render, so a ref-backed predicate picks up index updates. */
    validatePath?: (path: string) => boolean;
  },
): IDisposable {
  // Cache the shape-only scan per row text so scroll/hover over unchanged
  // rows doesn't re-run the regex. `validatePath` is a live gate (the
  // index can change), so it's applied fresh below on the cached matches,
  // never cached.
  const scanLine = createMemoizedScanner((text) => findFilePathMatches(text));
  const provider: ILinkProvider = {
    provideLinks(bufferLineNumber, callback) {
      const text = readWrappedLine(term, bufferLineNumber);
      if (!text) {
        callback(undefined);
        return;
      }
      const shapeMatches = scanLine(text);
      const matches = opts.validatePath
        ? shapeMatches.filter((m) => opts.validatePath!(m.text))
        : shapeMatches;
      if (matches.length === 0) {
        callback(undefined);
        return;
      }
      const cols = term.cols;
      const links: ILink[] = [];
      for (const m of matches) {
        // Convert 0-based char offsets in the wrapped line text to
        // xterm 1-based cell coordinates {x, y}, accounting for the
        // line wrapping at `cols` cells per row. (We assume one cell
        // per char — wide CJK chars will misalign by a cell here, an
        // acceptable v1 limitation; the click still resolves the
        // path correctly because we pass `m.text` straight through.)
        const startCell = m.start;
        const endCell = m.end - 1; // inclusive end cell
        const start = {
          x: (startCell % cols) + 1,
          y: bufferLineNumber + Math.floor(startCell / cols),
        };
        const end = {
          x: (endCell % cols) + 1,
          y: bufferLineNumber + Math.floor(endCell / cols),
        };
        const activation: FilePathLinkActivation = {
          text: m.text,
          line: m.line,
          column: m.column,
        };
        links.push({
          range: { start, end },
          text: text.slice(m.start, m.end),
          activate: () => opts.onActivate(activation),
          hover: opts.getHover
            ? (_e, _t) => { /* xterm renders default tooltip via `text`; hook reserved */ }
            : undefined,
        });
      }
      callback(links);
    },
  };
  return term.registerLinkProvider(provider);
}

/**
 * Read the full text of an xterm buffer line including subsequent
 * wrapped continuations. xterm calls `provideLinks` for every visual
 * row, but a path may straddle a wrap boundary; we coalesce so the
 * match offsets are computed against the contiguous string.
 *
 * Returns null if the line is not a wrap-start (i.e. the previous
 * line was wrapped into this one) — the host is the wrap-start row,
 * and the link ranges are computed from there.
 */
function readWrappedLine(term: Terminal, bufferLineNumber: number): string | null {
  const buffer = term.buffer.active;
  // xterm's bufferLineNumber is 1-based; getLine() is 0-based.
  const idx0 = bufferLineNumber - 1;
  const line = buffer.getLine(idx0);
  if (!line) return null;
  // If THIS line is a wrap-continuation of the previous line, skip —
  // the link will be discovered when xterm asks about the wrap-start
  // row. (`isWrapped` is true on continuation rows.)
  if (line.isWrapped) return null;
  let text = line.translateToString(true);
  for (let i = idx0 + 1; i < buffer.length; i++) {
    const next = buffer.getLine(i);
    if (!next || !next.isWrapped) break;
    text += next.translateToString(true);
  }
  return text;
}
