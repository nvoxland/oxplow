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

import { LOCAL_FS } from "../filesystem.js";

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
export function parseUri(text: string): ParsedUri | null {
  const p = parse(text);
  return "uri" in p ? p.uri : null;
}

/** Why `text` isn't one of oxplow's URIs (the fixture's reason), or null. */
export function uriProblem(text: string): Reason | null {
  const p = parse(text);
  return "reason" in p ? p.reason : null;
}

/** The canonical string for `u`. */
export function formatUri(u: ParsedUri): string {
  for (const k of [...Object.keys(u.params), ...Object.keys(u.view)]) {
    if (!KEY.test(k)) throw new Error(`\`${k}\` isn't a URI param key`);
  }
  return `oxplow:${u.resource.extension}/${u.resource.name}${writePairs("?", u.params)}${writePairs("#", u.view)}`;
}

/** Whether `text` is one of oxplow's URIs (the scheme, not the grammar):
 *  what a link or a drop is checked with before it is parsed. */
export function isOxplowScheme(text: string): boolean {
  return text.startsWith("oxplow:");
}

/** Build `resource` (`oxplow/file`) with `params` and `view`; a null or
 *  undefined value is left out. */
export function resourceUri(resource: string, params: Params = {}, view: Params = {}): string {
  const slash = resource.indexOf("/");
  const extension = resource.slice(0, slash);
  const name = resource.slice(slash + 1);
  if (slash < 0 || !EXTENSION.test(extension) || !NAME.test(name)) throw new Error(`\`${resource}\` isn't <extension>/<resource>`);
  return formatUri({ resource: { extension, name }, params: present(params), view: present(view) });
}

export type ParamValue = string | number | null | undefined;
export type Params = Record<string, ParamValue>;

function present(params: Params): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(params)) if (v !== null && v !== undefined) out[k] = String(v);
  return out;
}

/** The canonical URI without its view: two URIs name the same resource
 *  exactly when their identities are equal. Null when `text` isn't one. */
export function identityOf(text: string): string | null {
  const u = parseUri(text);
  return u ? formatUri({ ...u, view: {} }) : null;
}

/** `<extension>/<name>` of `text` (`oxplow/wiki`), or null when it isn't
 *  one of oxplow's URIs. */
export function typeOf(text: string): string | null {
  const u = parseUri(text);
  return u ? `${u.resource.extension}/${u.resource.name}` : null;
}

/** Whether `text` is a URI of `resource`. */
export function isUriOfType(text: string, resource: string): boolean {
  return typeOf(text) === resource;
}

/** `text`'s param `key` (decoded), or null. */
export function paramOf(text: string, key: string): string | null {
  return parseUri(text)?.params[key] ?? null;
}

/** `text`'s view param `key` (decoded), or null. */
export function viewOf(text: string, key: string): string | null {
  return parseUri(text)?.view[key] ?? null;
}

/** Where oxplow's own resources keep the param that names one: the rest
 *  take `id`, and a contribution (a page, a metric) its own name
 *  (`oxplow/page?page=acme/x`). A resource the extension declares itself
 *  keeps its name in `id`. */
const KEY_PARAM: Record<string, string> = {
  commit: "sha",
  wiki: "slug",
  file: "path",
  dir: "path",
  branch: "name",
  ai_model: "name",
  extension: "name",
  consumer: "name",
  config: "key",
  nudge: "kind",
  dataset: "dataset",
  measure: "measure",
  metric: "metric",
  dimension: "dimension",
  collector: "collector",
  event_type: "event_type",
  ref_kind: "ref_kind",
  command: "command",
  job: "job",
  ai_profile: "ai_profile",
  effect: "effect",
  advisory: "advisory",
  skill: "skill",
  implementation: "implementation",
  provider: "provider",
  page: "page",
  panel: "panel",
};

/** The param that names `resource` (`oxplow/wiki` → `slug`). */
export function keyParamOf(resource: string): string {
  const [extension, name] = resource.split("/");
  return (extension === "oxplow" && name !== undefined ? KEY_PARAM[name] : undefined) ?? "id";
}

/** `text` with only its key param: the page it names whatever it
 *  shows and however (`oxplow/page?page=tasks`) — page memory's scope. Null
 *  when `text` isn't a URI. */
export function keyIdentityOf(text: string): string | null {
  const u = parseUri(text);
  if (!u) return null;
  const key = keyParamOf(`${u.resource.extension}/${u.resource.name}`);
  const value = u.params[key];
  return formatUri({ resource: u.resource, params: value === undefined ? {} : { [key]: value }, view: {} });
}

/** The key param of `text` when it is a URI of `resource`, else null. */
export function idOf(text: string, resource: string): string | null {
  const u = parseUri(text);
  if (!u || `${u.resource.extension}/${u.resource.name}` !== resource) return null;
  return u.params[keyParamOf(resource)] ?? null;
}

/** A work item's list and its id in that list; null for anything else. */
export function workItemOf(text: string): { provider: string; id: string } | null {
  const u = parseUri(text);
  if (!u || typeOf(text) !== "oxplow/work_item") return null;
  const { provider, id } = u.params;
  return provider && id ? { provider, id } : null;
}

/** `text`'s key param, whatever its resource (`thr3` of a thread, the slug
 *  of a wiki page); null when it isn't a URI or lacks it. */
export function keyOf(text: string): string | null {
  const resource = typeOf(text);
  return resource === null ? null : idOf(text, resource);
}

/** The URI of `resource` named by its key param. For a resource known
 *  only at run time; one known at the call site has a builder below. */
export function keyedUri(resource: string, id: string): string {
  return resourceUri(resource, { [keyParamOf(resource)]: id });
}

/** `uri` as a markdown link target: its parentheses escaped, which a
 *  canonical URI carries raw and a `[label](…)` would end at. Read back
 *  by `parseUri` (`%28` decodes). */
export function linkTarget(uri: string): string {
  return uri.replace(/\(/g, "%28").replace(/\)/g, "%29");
}

/** A resource's short name for the chrome's tables (icons, labels): oxplow's
 *  own drop their namespace (`oxplow/file` → `file`), an extension's keep
 *  theirs (`acme/pr`). */
export function shortResource(resource: string): string {
  return resource.startsWith("oxplow/") ? resource.slice("oxplow/".length) : resource;
}

/** Whether `text` is a URI of another scheme (the web, mail): a resource
 *  oxplow hands to that scheme's renderer. */
export function isExternal(text: string): boolean {
  if (isOxplowScheme(text)) return false;
  try {
    new URL(text);
    return /^[A-Za-z][A-Za-z0-9+.-]*:/.test(text);
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------------------
// Builders. A URI's owner spells it and everyone else carries it: this file
// is the one place in the desktop that writes `oxplow:` (`uri.guard.test.ts`
// fails on a hand-built or hand-parsed one anywhere else). A caller names a
// resource through its builder here.
// ---------------------------------------------------------------------------

/** A thread (`thr3`). */
export const threadUri = (id: string): string => resourceUri("oxplow/thread", { id });
/** A stream (`str1`). */
export const streamUri = (id: string): string => resourceUri("oxplow/stream", { id });
/** An effort (`eff12`). */
export const effortUri = (id: string): string => resourceUri("oxplow/effort", { id });
/** A wiki comment (`cmt12`). */
export const commentUri = (id: string): string => resourceUri("oxplow/comment", { id });
/** An agent session (`ses3`). */
export const agentSessionUri = (id: string): string => resourceUri("oxplow/agent_session", { id });
/** A wiki page (`architecture`). */
export const wikiUri = (slug: string): string => resourceUri("oxplow/wiki", { slug });
/** A logged event, by its id. */
export const eventUri = (id: string): string => resourceUri("oxplow/event", { id });
/** A command proposal (`command_proposal.id`). */
export const proposalUri = (id: string | number): string => resourceUri("oxplow/proposal", { id });
/** A `.oxplow/project.yaml` key (`snapshotRetentionDays`). */
export const configUri = (key: string): string => resourceUri("oxplow/config", { key });
/** A snapshot, by its id. */
export const snapshotUri = (id: string | number): string => resourceUri("oxplow/snapshot", { id });
/** An agent turn, by its id. */
export const turnUri = (id: string | number): string => resourceUri("oxplow/turn", { id });
/** A commit, by its sha. */
export const commitUri = (sha: string): string => resourceUri("oxplow/commit", { sha });
/** A work item, whichever list holds it. */
export const workItemUri = (provider: string, id: string): string => resourceUri("oxplow/work_item", { provider, id });
/** A command, by its bus id. */
export const commandUri = (id: string): string => resourceUri("oxplow/command", { command: id });
/** A panel the rail or a view remembers things about. */
export const panelUri = (panel: string): string => resourceUri("oxplow/panel", { panel });

/** The `filesystem` param a URI carries: none for the local filesystem,
 *  the default, so one file has one identity. */
const filesystemParam = (filesystem: string | null): string | null => (filesystem === LOCAL_FS ? null : filesystem);

/** A file on `filesystem` (null: the local one) at a revision, the
 *  filesystem's own word for one (`git:HEAD`, `snap:…`; none: its default,
 *  the working tree). */
export const fileUri = (path: string, rev: string | null = null, filesystem: string | null = null): string =>
  resourceUri("oxplow/file", { path, rev, filesystem: filesystemParam(filesystem) });
/** A directory, at a revision. */
export const dirUri = (path: string, rev: string | null = null, filesystem: string | null = null): string =>
  resourceUri("oxplow/dir", { path, rev, filesystem: filesystemParam(filesystem) });

/** A file's lines (`#lines=10-20`; one line is `10-10`): what Ask About
 *  This names for an editor or diff selection. */
export function fileLinesUri(
  path: string,
  start: number,
  end: number,
  rev: string | null = null,
  filesystem: string | null = null,
): string {
  return resourceUri("oxplow/file", { path, rev, filesystem: filesystemParam(filesystem) }, { lines: `${start}-${end}` });
}

/** A shell route: `oxplow/page?page=<name>`, the params that say what it
 *  shows and its view (`#`: a filter, a pick). */
export function pageUri(page: string, params: Params = {}, view: Params = {}): string {
  return resourceUri("oxplow/page", { ...params, page }, view);
}

/** An extension's page: the contribution `page=<extension>/<page>`. */
export function extPageUri(extension: string, page: string, params: Params = {}, view: Params = {}): string {
  return pageUri(`${extension}/${page}`, params, view);
}

/** A metric's page (`oxplow.todos`) and its view (range, branch, …). */
export function metricUri(key: string, params: Params = {}, view: Params = {}): string {
  return resourceUri("oxplow/metric", { metric: key, ...params }, view);
}

/** `text` showing `view` (its `#`), its identity unchanged; a null or
 *  undefined value is left out. `text` as it is when it isn't a URI. */
export function withUriView(text: string, view: Params): string {
  const u = parseUri(text);
  return u ? formatUri({ ...u, view: present(view) }) : text;
}

/** `text`'s view (its `#`'s params, decoded); empty when it has none. */
export function viewParamsOfUri(text: string): Record<string, string> {
  return { ...(parseUri(text)?.view ?? {}) };
}

/** An extension's collector (`<owner>/<id>`). */
export function collectorUri(owner: string, id: string): string {
  return resourceUri("oxplow/collector", { collector: `${owner}/${id}` });
}

/** A contribution's URI as its resource (`oxplow/page`), its extension
 *  (null for a globally named kind) and its id; null for anything else. */
const CONTRIBUTIONS = new Set([
  "dataset", "measure", "metric", "dimension", "collector", "event_type", "ref_kind", "command", "job",
  "ai_profile", "effect", "advisory", "skill", "implementation", "provider", "page", "panel",
]);
const GLOBAL_CONTRIBUTIONS = new Set(["command", "measure", "metric", "dimension", "event_type", "ref_kind"]);

export function contributionOf(text: string): { kind: string; extension: string | null; id: string } | null {
  const u = parseUri(text);
  if (!u || u.resource.extension !== "oxplow" || !CONTRIBUTIONS.has(u.resource.name)) return null;
  const id = u.params[u.resource.name];
  if (id === undefined) return null;
  if (GLOBAL_CONTRIBUTIONS.has(u.resource.name)) return { kind: u.resource.name, extension: null, id };
  const slash = id.indexOf("/");
  if (slash <= 0 || slash === id.length - 1) return null;
  return { kind: u.resource.name, extension: id.slice(0, slash), id: id.slice(slash + 1) };
}
