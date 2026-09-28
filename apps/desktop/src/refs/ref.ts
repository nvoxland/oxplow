// The canonical ref grammar: `<kind>:<id>[@<rev>][#<frag>]`.
// See .context/refs.md. Rust (`oxplow-domain::refs::grammar`) is the
// source of truth; `ref.test.ts` asserts this parser against the same
// golden fixture so the two can't drift.
//
// Only `@`, `#` and `%` are reserved inside an id, so refs stay readable:
// `work_item:oxplow:tsk42` and `lens:acme/blocked?stream_id=2` parse
// without escaping. The first `:` ends the kind; the first `#` ends the
// id-and-rev (fragments aren't encoded); the first unescaped `@` before
// that starts the rev. A rev is always `<kind>:<value>` (`git:HEAD`,
// `snap:01J9…`).

export interface CanonicalRef {
  kind: string;
  /** Decoded: an `@` in a file name is a real `@` here. */
  id: string;
  rev: string | null;
  frag: string | null;
}

const KIND_RE = /^[a-z][a-z0-9_]*$/;
const RESERVED = new Set(["@", "#", "%"]);

export function isValidKind(kind: string): boolean {
  return KIND_RE.test(kind);
}

/** `%XX` escapes decoded; a raw reserved character is an error (null). */
function decode(s: string): string | null {
  let out = "";
  for (let i = 0; i < s.length; i++) {
    const c = s[i]!;
    if (c === "%") {
      const hex = s.slice(i + 1, i + 3);
      if (hex.length !== 2 || !/^[0-9a-fA-F]{2}$/.test(hex)) return null;
      out += String.fromCharCode(parseInt(hex, 16));
      i += 2;
    } else if (RESERVED.has(c)) {
      return null;
    } else {
      out += c;
    }
  }
  return out;
}

function encode(s: string): string {
  let out = "";
  for (const c of s) {
    out += RESERVED.has(c) ? `%${c.charCodeAt(0).toString(16).toUpperCase().padStart(2, "0")}` : c;
  }
  return out;
}

/** A revision is `<kind>:<value>` with a valid kind and a non-empty value. */
function isValidRev(rev: string): boolean {
  const colon = rev.indexOf(":");
  if (colon <= 0) return false;
  return isValidKind(rev.slice(0, colon)) && rev.length > colon + 1;
}

/** Parse a canonical ref; `null` when the text isn't one. */
export function parseRef(text: string): CanonicalRef | null {
  if (!text) return null;
  const colon = text.indexOf(":");
  if (colon === -1) return null;
  const kind = text.slice(0, colon);
  if (!isValidKind(kind)) return null;
  const rest = text.slice(colon + 1);
  const hash = rest.indexOf("#");
  const beforeFrag = hash === -1 ? rest : rest.slice(0, hash);
  const frag = hash === -1 ? null : rest.slice(hash + 1);
  if (frag === "") return null;
  const at = beforeFrag.indexOf("@");
  const idEnc = at === -1 ? beforeFrag : beforeFrag.slice(0, at);
  const revEnc = at === -1 ? null : beforeFrag.slice(at + 1);
  if (idEnc === "") return null;
  const id = decode(idEnc);
  if (id === null) return null;
  let rev: string | null = null;
  if (revEnc !== null) {
    if (revEnc === "") return null;
    rev = decode(revEnc);
    if (rev === null || !isValidRev(rev)) return null;
  }
  return { kind, id, rev, frag };
}

/** The canonical string form; re-encodes the reserved characters. */
export function formatRef(r: CanonicalRef): string {
  let out = `${r.kind}:${encode(r.id)}`;
  if (r.rev !== null) out += `@${encode(r.rev)}`;
  if (r.frag !== null) out += `#${r.frag}`;
  return out;
}

/** Build and format in one step. Throws on a bad kind, since a caller
 *  constructing a ref from a literal kind has a programming error. */
export function ref(kind: string, id: string, rev: string | null = null, frag: string | null = null): string {
  if (!isValidKind(kind)) throw new Error(`bad ref kind \`${kind}\``);
  return formatRef({ kind, id, rev, frag });
}

/** The kind of a ref string, or null when it isn't a canonical ref. */
export function kindOf(text: string): string | null {
  return parseRef(text)?.kind ?? null;
}
