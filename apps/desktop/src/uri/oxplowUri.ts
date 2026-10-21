// oxplow's URIs (.context/resources.md): one scheme, the resource as
// `<extension>/<name>`, its identity as named params, and the renderer's
// view as named params after `#`:
//
//   oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD#lines=10-20
//
// Keys are unique and their order doesn't matter: the canonical form sorts
// them. The view is never part of the identity (`identityOf`). The Rust
// parser (`oxplow_domain::uri::oxplow_uri`) is the same grammar; both
// assert crates/oxplow-domain/tests/fixtures/uri.json. The string is split
// by hand, not by `URL`/`URLSearchParams`, which read `+` as a space and
// normalize paths.

/** One of oxplow's URIs, canonical. Only this module makes one. */
export type OxplowUri = string & { readonly __oxplowUri: unique symbol };

export interface ResourceName {
  extension: string;
  name: string;
}

export interface ParsedUri {
  resource: ResourceName;
  /** Which resource it is (decoded). */
  params: Record<string, string>;
  /** How a renderer shows it (decoded). */
  view: Record<string, string>;
}

const EXTENSION = /^[a-z][a-z0-9-]*$/;
const NAME = /^[a-z][a-z0-9_]*$/;
const KEY = /^[A-Za-z][A-Za-z0-9_.-]*$/;
/** A byte a canonical key or value carries as itself. */
const RAW = /^[A-Za-z0-9\-._~/:?,@!$'()*;]$/;

type Reason =
  | "empty"
  | "not_oxplow"
  | "authority"
  | "bad_resource"
  | "bad_param"
  | "duplicate_param"
  | "bad_escape"
  | "not_utf8"
  | "unescaped";

type Parse = { ok: true; uri: ParsedUri } | { ok: false; reason: Reason };

function parse(text: string): Parse {
  if (text === "") return { ok: false, reason: "empty" };
  if (!text.startsWith("oxplow:")) return { ok: false, reason: "not_oxplow" };
  const rest = text.slice("oxplow:".length);
  if (rest.startsWith("//")) return { ok: false, reason: "authority" };
  const hash = rest.indexOf("#");
  const beforeView = hash < 0 ? rest : rest.slice(0, hash);
  const viewText = hash < 0 ? "" : rest.slice(hash + 1);
  const q = beforeView.indexOf("?");
  const path = q < 0 ? beforeView : beforeView.slice(0, q);
  const query = q < 0 ? "" : beforeView.slice(q + 1);
  const slash = path.indexOf("/");
  const extension = slash < 0 ? "" : path.slice(0, slash);
  const name = slash < 0 ? "" : path.slice(slash + 1);
  if (!EXTENSION.test(extension) || !NAME.test(name)) return { ok: false, reason: "bad_resource" };
  const params = pairs(query);
  if ("reason" in params) return { ok: false, reason: params.reason };
  const view = pairs(viewText);
  if ("reason" in view) return { ok: false, reason: view.reason };
  return { ok: true, uri: { resource: { extension, name }, params: params.pairs, view: view.pairs } };
}

function pairs(text: string): { ok: true; pairs: Record<string, string> } | { ok: false; reason: Reason } {
  const out: Record<string, string> = {};
  if (text === "") return { ok: true, pairs: out };
  for (const pair of text.split("&")) {
    const eq = pair.indexOf("=");
    const key = eq < 0 ? "" : pair.slice(0, eq);
    if (!KEY.test(key)) return { ok: false, reason: "bad_param" };
    const value = decode(pair.slice(eq + 1));
    if (typeof value !== "string") return { ok: false, reason: value.reason };
    if (Object.prototype.hasOwnProperty.call(out, key)) return { ok: false, reason: "duplicate_param" };
    out[key] = value;
  }
  return { ok: true, pairs: out };
}

/** A value's `%XX` escapes decoded; a raw character must be one a
 *  canonical value carries, or `+` (a plus, never a space). */
function decode(value: string): string | { ok: false; reason: Reason } {
  const bytes: number[] = [];
  for (let i = 0; i < value.length; ) {
    const c = value[i]!;
    if (c === "%") {
      const hex = value.slice(i + 1, i + 3);
      if (!/^[0-9A-Fa-f]{2}$/.test(hex)) return { ok: false, reason: "bad_escape" };
      bytes.push(parseInt(hex, 16));
      i += 3;
    } else if (RAW.test(c) || c === "+") {
      bytes.push(c.charCodeAt(0));
      i += 1;
    } else {
      return { ok: false, reason: "unescaped" };
    }
  }
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(bytes));
  } catch {
    return { ok: false, reason: "not_utf8" };
  }
}

function encode(value: string): string {
  let out = "";
  for (const b of new TextEncoder().encode(value)) {
    const c = String.fromCharCode(b);
    out += b < 0x80 && RAW.test(c) ? c : `%${b.toString(16).toUpperCase().padStart(2, "0")}`;
  }
  return out;
}

function writePairs(lead: string, pairs: Record<string, string>): string {
  const keys = Object.keys(pairs).sort();
  return keys.length === 0 ? "" : lead + keys.map((k) => `${k}=${encode(pairs[k]!)}`).join("&");
}

/** `text` parsed, or null when it isn't one of oxplow's URIs. */
export function parseOxplowUri(text: string): ParsedUri | null {
  const p = parse(text);
  return "uri" in p ? p.uri : null;
}

/** Why `text` isn't one of oxplow's URIs (the fixture's reason), or null. */
export function uriProblem(text: string): Reason | null {
  const p = parse(text);
  return "reason" in p ? p.reason : null;
}

/** The canonical string for `u`. */
export function formatUri(u: ParsedUri): OxplowUri {
  for (const k of [...Object.keys(u.params), ...Object.keys(u.view)]) {
    if (!KEY.test(k)) throw new Error(`\`${k}\` isn't a URI param key`);
  }
  return `oxplow:${u.resource.extension}/${u.resource.name}${writePairs("?", u.params)}${writePairs("#", u.view)}` as OxplowUri;
}

/** Build `resource` (`oxplow/file`) with `params` and `view`. */
export function oxplowUri(resource: string, params: Record<string, string> = {}, view: Record<string, string> = {}): OxplowUri {
  const slash = resource.indexOf("/");
  const extension = resource.slice(0, slash);
  const name = resource.slice(slash + 1);
  if (slash < 0 || !EXTENSION.test(extension) || !NAME.test(name)) throw new Error(`\`${resource}\` isn't <extension>/<resource>`);
  return formatUri({ resource: { extension, name }, params, view });
}

/** The canonical URI without its view: two URIs name the same resource
 *  exactly when their identities are equal. Null when `text` isn't one. */
export function identityOf(text: string): OxplowUri | null {
  const u = parseOxplowUri(text);
  return u ? formatUri({ ...u, view: {} }) : null;
}

/** `<extension>/<name>` of `text`, or null. */
export function resourceOf(text: string): string | null {
  const u = parseOxplowUri(text);
  return u ? `${u.resource.extension}/${u.resource.name}` : null;
}

/** `text`'s param `key` (decoded), or null. */
export function paramOf(text: string, key: string): string | null {
  return parseOxplowUri(text)?.params[key] ?? null;
}

/** Whether `text` is a URI of another scheme (the web, mail): a resource
 *  oxplow hands to that scheme's renderer. */
export function isExternal(text: string): boolean {
  if (text.startsWith("oxplow:")) return false;
  try {
    new URL(text);
    return /^[A-Za-z][A-Za-z0-9+.-]*:/.test(text);
  } catch {
    return false;
  }
}
