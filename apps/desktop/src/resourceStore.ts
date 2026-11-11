/**
 * The resources the vocabulary knows (`.context/resources.md`), as one
 * process-wide list, with what reads it and no React: how a `[[…]]` names
 * one, and the query that titles one. `resources.ts` loads the list and
 * lets components subscribe to it; this module stays free of React so a
 * component frame's markdown (`markdownTree`) and the test harness can use
 * it.
 */
import { identityOf, resourceUri } from "./uri/uri.js";
import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/** One param of a resource: whether every URI carries it and the anchored
 *  pattern its value matches. */
export interface ResourceParam {
  required: boolean;
  pattern: string | null;
}

/** One resource, as `v_resource` lists it. */
export interface Resource {
  /** `<extension>/<name>` (`oxplow/file`, `acme/pr`). */
  resource: string;
  extension: string;
  /** What one is called. */
  label: string;
  /** Its params, by key. */
  params: Record<string, ResourceParam>;
  /** It takes params beyond its own (a page's, a viz node's). */
  additional: boolean;
  /** The view whose rows name each one, by URI. */
  resolve: string | null;
  /** The column of `resolve` that titles one (`title` when absent). */
  title: string | null;
  searchable: string | null;
  icon: string | null;
  /** Its `[[prefix:value]]` sugar: the prefix and the param it sets. */
  wikilink: { prefix: string; param: string } | null;
}

export function resourcesFromResult(result: SqlQueryResult): Resource[] {
  return result.rows.map((row) => {
    const at = (name: string) => row[result.columns.indexOf(name)] ?? null;
    const text = (name: string) => String(at(name) ?? "");
    const maybe = (name: string) => {
      const v = at(name);
      return v === null || v === undefined || v === "" ? null : String(v);
    };
    const json = <T>(name: string, fallback: T): T => {
      try {
        const parsed: unknown = JSON.parse(text(name) || "null");
        return parsed === null || parsed === undefined ? fallback : (parsed as T);
      } catch {
        return fallback;
      }
    };
    const params: Record<string, ResourceParam> = {};
    for (const [key, decl] of Object.entries(json<Record<string, { required?: boolean; pattern?: string | null }>>("params", {}))) {
      params[key] = { required: decl.required === true, pattern: decl.pattern ?? null };
    }
    const wikilink = json<{ prefix?: string; param?: string } | null>("wikilink", null);
    return {
      resource: text("resource"),
      extension: text("extension"),
      label: text("label"),
      params,
      additional: at("additional") === 1 || at("additional") === true,
      resolve: maybe("resolve"),
      title: maybe("title"),
      searchable: maybe("searchable"),
      icon: maybe("icon"),
      wikilink: wikilink?.prefix && wikilink.param ? { prefix: wikilink.prefix, param: wikilink.param } : null,
    };
  });
}

let current: ReadonlyMap<string, Resource> = new Map();
const listeners = new Set<() => void>();

/** Replace the list (the loader; tests). */
export function setResources(resources: readonly Resource[]): void {
  current = new Map(resources.map((r) => [r.resource, r]));
  for (const l of listeners) l();
}

/** Whether the list has been read: before it, a resource that isn't oxplow's
 *  is not yet known to be missing. */
export function resourcesLoaded(): boolean {
  return current.size > 0;
}

/** A resource by name (`oxplow/file`), or null (unknown, or the list isn't
 *  loaded yet). */
export function resourceInfo(resource: string): Resource | null {
  return current.get(resource) ?? null;
}



/** Whether `id` is a value `param` of `resource` may hold. The pattern runs
 *  in JS's backtracking engine: the load check keeps it in one explicit
 *  ASCII form both engines read alike, with no repeats that could share
 *  characters unfenced, so no id makes it backtrack. */
function acceptsParam(resource: Resource, param: string, id: string): boolean {
  const pattern = resource.params[param]?.pattern;
  if (!pattern) return true;
  try {
    return new RegExp(pattern).test(id);
  } catch {
    return false;
  }
}

/**
 * The URI a `[[…]]` interior names through a resource's sugar: a declared
 * `wikilink:` prefix (`pr:12` → `oxplow:acme/pr?id=12`), or — for a
 * resource of oxplow's whose one param names it — its own name
 * (`turn:trn3`, `acme:x`). Null when it names none, or the value doesn't
 * match the param's pattern, as the backend's wikilink reader checks.
 */
export function extensionWikilinkRef(interior: string): string | null {
  const colon = interior.indexOf(":");
  if (colon <= 0) return null;
  const head = interior.slice(0, colon).toLowerCase();
  const id = interior.slice(colon + 1).trim();
  if (!id) return null;
  const all = [...current.values()];
  // The backend's order: a declared prefix, then a resource's own name.
  const declared = all.find((r) => r.wikilink?.prefix === head);
  if (declared?.wikilink) {
    const { param } = declared.wikilink;
    return acceptsParam(declared, param, id) ? resourceUri(declared.resource, { [param]: id }) : null;
  }
  const named = current.get(`oxplow/${head}`);
  const keys = named ? Object.keys(named.params) : [];
  if (named && keys.length === 1 && named.params[keys[0]!]!.required) {
    return acceptsParam(named, keys[0]!, id) ? resourceUri(named.resource, { [keys[0]!]: id }) : null;
  }
  return null;
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The query that reads `ref`'s title from its resource's dataset, or
 *  null when its resource names none. View and column names are oxplow's
 *  (`v_<extension>_<dataset>`, a dataset's column), never text a person
 *  typed; checked anyway, since they're spliced into the query. A
 *  dataset's `uri` is the identity: the ref's view (`#part=…`) is the
 *  page's. */
export function extensionTitleQuery(ref: string, resource: Resource | undefined): { sql: string; params: [string] } | null {
  const view = resource?.resolve ?? null;
  const column = resource?.title ?? "title";
  if (!view || !/^v_[a-z0-9_]+$/.test(view) || !/^[a-z][a-z0-9_]*$/.test(column)) return null;
  return { sql: `SELECT ${column} FROM ${view} WHERE uri = ?1 LIMIT 1`, params: [identityOf(ref) ?? ref] };
}

/** The list as it is now (`useResources` subscribes to it). */
export function currentResources(): ReadonlyMap<string, Resource> {
  return current;
}
