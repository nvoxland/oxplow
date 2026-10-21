// The canonical uri grammar: `<kind>:<id>[@<rev>][#<frag>]`.
// See .context/resources.md. Rust (`oxplow_domain::uri::grammar`) is the
// source of truth; `uri.test.ts` asserts this parser against the same
// golden fixture so the two can't drift.
//
// Only `@`, `#` and `%` are reserved inside an id, so refs stay readable:
// `work_item:oxplow:tsk42` and `lens:acme/blocked?stream_id=2` parse
// without escaping. The first `:` ends the kind; the first `#` ends the
// id-and-rev (fragments aren't encoded); the first unescaped `@` before
// that starts the rev. A rev is always `<kind>:<value>` (`git:HEAD`,
// `snap:01J9…`).

export interface Uri {
  kind: string;
  /** Decoded: an `@` in a file name is a real `@` here. */
  id: string;
  rev: string | null;
  frag: string | null;
}

const KIND_RE = /^[a-z][a-z0-9_]*$/;
const RESERVED = new Set(["@", "#", "%"]);

export function isValidType(kind: string): boolean {
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

/** Percent-encode the three reserved characters of an id (or rev)
 *  segment; everything else stays raw so refs remain readable. */
export function escapeId(s: string): string {
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
  return isValidType(rev.slice(0, colon)) && rev.length > colon + 1;
}

/** Parse a canonical uri; `null` when the text isn't one. */
export function parseUri(text: string): Uri | null {
  if (!text) return null;
  const colon = text.indexOf(":");
  if (colon === -1) return null;
  const kind = text.slice(0, colon);
  if (!isValidType(kind)) return null;
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
export function formatUri(r: Uri): string {
  let out = `${r.kind}:${escapeId(r.id)}`;
  if (r.rev !== null) out += `@${escapeId(r.rev)}`;
  if (r.frag !== null) out += `#${r.frag}`;
  return out;
}

/** Build and format in one step. Throws on a bad kind, since a caller
 *  constructing a uri from a literal kind has a programming error. */
export function uri(kind: string, id: string, rev: string | null = null, frag: string | null = null): string {
  if (!isValidType(kind)) throw new Error(`bad uri kind \`${kind}\``);
  return formatUri({ kind, id, rev, frag });
}

/** A file's lines (`file:src/a.rs#L10-20`, `file:src/a.rs@git:abc#L3`
 *  at a revision): what Ask About This names for an editor or diff
 *  selection. */
export function fileLinesUri(path: string, start: number, end: number, rev: string | null = null): string {
  return uri("file", path, rev, start === end ? `L${start}` : `L${start}-${end}`);
}

/** The kind of a uri string, or null when it isn't a canonical uri. */
export function typeOf(text: string): string | null {
  return parseUri(text)?.kind ?? null;
}

// ---------------------------------------------------------------------------
// Builders. A uri's owner spells it and everyone else carries it: this file
// is the one place in the desktop that writes a kind prefix
// (`refs.guard.test.ts` fails on a hand-built or hand-parsed one anywhere
// else). A caller names the kind through a builder here, or `uriOf`.
// ---------------------------------------------------------------------------

/** `<kind>:<id>` for any kind. */
export function uriOf(kind: string, id: string): string {
  return uri(kind, id);
}

/** The id of `text` when it is a uri of `kind`, else null. */
export function idOf(text: string, kind: string): string | null {
  const parsed = parseUri(text);
  return parsed !== null && parsed.kind === kind ? parsed.id : null;
}

/** Whether `text` is a uri of `kind`. */
export function isUriOfType(text: string, kind: string): boolean {
  return typeOf(text) === kind;
}

/** A thread (`thr3` → `thread:thr3`). */
export const threadUri = (id: string): string => uriOf("thread", id);
/** A stream (`str1` → `stream:str1`). */
export const streamUri = (id: string): string => uriOf("stream", id);
/** An effort (`eff12` → `effort:eff12`). */
export const effortUri = (id: string): string => uriOf("effort", id);
/** A wiki comment (`cmt12` → `comment:cmt12`). */
export const commentUri = (id: string): string => uriOf("comment", id);
/** An agent session (`ses3` → `agent_session:ses3`). */
export const agentSessionUri = (id: string): string => uriOf("agent_session", id);
/** A wiki page (`architecture` → `wiki:architecture`). */
export const wikiUri = (slug: string): string => uriOf("wiki", slug);
/** A logged event, by its id. */
export const eventUri = (id: string): string => uriOf("event", id);
/** A command proposal (`command_proposal.id`). */
export const proposalUri = (id: string | number): string => uriOf("proposal", String(id));
/** A `.oxplow/project.yaml` key (`config:snapshotRetentionDays`). */
export const configUri = (key: string): string => uriOf("config", key);
/** A work item, whichever list holds it (`work_item:<provider>:<id>`). */
export const workItemUri = (provider: string, id: string): string => uriOf("work_item", `${provider}:${id}`);

// Query params after a route name or a lens id: `?k=v&k=v`. Values are
// percent-encoded the way `URLSearchParams` reads them, but only the
// characters that would break the id are escaped — the query syntax
// (`&`, `=`, `+`, `%`) and the grammar's reserved `@` and `#` — so
// `path=src/a.ts&left=uri:abc` stays readable. The encoded text is a valid
// canonical uri of its kind (`page:` / `lens:`) as written, so it is parsed
// from the RAW text after the kind, not from `parseUri`'s decoded id (which
// would turn an escaped `&` back into a separator).

export type ParamValue = string | number | null | undefined;

function encodeParamValue(v: string): string {
  return encodeURIComponent(v)
    .replace(/%2F/gi, "/")
    .replace(/%3A/gi, ":")
    .replace(/%3F/gi, "?")
    .replace(/%2C/gi, ",")
    .replace(/%20/g, "+");
}

/** `k=v&k=v` in the given key order; `null`/`undefined` values are skipped. */
export function encodeParams(params: Record<string, ParamValue>): string {
  const parts: string[] = [];
  for (const [k, v] of Object.entries(params)) {
    if (v === null || v === undefined) continue;
    parts.push(`${encodeParamValue(k)}=${encodeParamValue(String(v))}`);
  }
  return parts.join("&");
}

/** `head` plus `params` as a query (after any it has). */
export function withParams(head: string, params: Record<string, ParamValue>): string {
  const qs = encodeParams(params);
  if (!qs) return head;
  return `${head}${head.includes("?") ? "&" : "?"}${qs}`;
}

/** The part of a `page:` / `lens:` uri's raw text after its kind, split
 *  into its (decoded) head and query. Null when `text` isn't of `kind`. */
export function splitTypeParams(text: string, kind: string): { head: string; params: URLSearchParams } | null {
  const raw = rawAfterType(text, kind);
  return raw === null ? null : splitParams(raw);
}

/** The raw text after `<kind>:`, undecoded; null when `text` isn't of `kind`. */
export function rawAfterType(text: string, kind: string): string | null {
  const colon = text.indexOf(":");
  return colon !== -1 && text.slice(0, colon) === kind ? text.slice(colon + 1) : null;
}

/** A raw `head?k=v` tail as its decoded head and query. */
export function splitParams(raw: string): { head: string; params: URLSearchParams } {
  const q = raw.indexOf("?");
  const head = q === -1 ? raw : raw.slice(0, q);
  return { head: decodeURIComponent(head), params: new URLSearchParams(q === -1 ? "" : raw.slice(q + 1)) };
}

/** A shell route: `page:<name>[?params]`. */
export function pageUri(name: string, params?: Record<string, ParamValue>): string {
  const head = `page:${name}`;
  return params ? withParams(head, params) : head;
}

/** An extension's collector: the contribution `collector:<extension>/<id>`. */
export function collectorUri(extension: string, id: string): string {
  return uriOf("collector", `${extension}/${id}`);
}

/** An extension's page: the contribution `page:<extension>/<page>[?params]`
 *  (a shell route has no slash). */
export function extPageUri(extension: string, page: string, params?: Record<string, ParamValue>): string {
  return pageUri(`${extension}/${page}`, params);
}

/** The kinds of contribution named globally (a command's bus id, a metric
 *  key): their refs carry no extension. Every other contribution's uri is
 *  `<kind>:<extension>/<id>` (`contribution::ContributionKind::global_name`). */
const GLOBAL_CONTRIBUTION_KINDS = new Set(["command", "measure", "metric", "dimension", "event_type", "ref_kind"]);

/** A contribution's uri as its kind, its extension (null for a globally
 *  named kind) and its id; null for a string that isn't one. */
export function contributionOf(text: string): { kind: string; extension: string | null; id: string } | null {
  const r = parseUri(text);
  if (!r) return null;
  if (GLOBAL_CONTRIBUTION_KINDS.has(r.kind)) return { kind: r.kind, extension: null, id: r.id };
  const slash = r.id.indexOf("/");
  if (slash <= 0 || slash === r.id.length - 1) return null;
  return { kind: r.kind, extension: r.id.slice(0, slash), id: r.id.slice(slash + 1) };
}

/** A metric's page: `metric:<key>[?params]` — its view (range, branch, …)
 *  as a query, the way a lens's params are. */
export function metricUri(key: string, params?: Record<string, ParamValue>): string {
  const head = `metric:${escapeId(key)}`;
  return params ? withParams(head, params) : head;
}

/** A lens: `lens:<extension>/<slug>[?params]`; params are the caller's,
 *  in the order given. */
export function lensUri(lensId: string, params?: Record<string, ParamValue>): string {
  const head = `lens:${escapeId(lensId)}`;
  return params ? withParams(head, params) : head;
}
