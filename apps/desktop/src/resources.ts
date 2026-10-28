/**
 * Every resource the vocabulary knows (`.context/resources.md`), read
 * from `v_resource`: oxplow's and those extensions declare. How the chrome
 * draws one (an extension's icon and label), how a `[[…]]` names one, and
 * which model holds it. What opens one is the page that renders it
 * (`pages/renderers.ts`).
 *
 * One process-wide list, loaded by `useResourcesLoader` (mounted once, in
 * `App`) and re-read when `v_resource` changes; the pure helpers
 * (`pageKindIconComponent`, `refFromTabId`, `preprocessWikilinks`) read
 * the current list, and components that render refs of extensions' kinds subscribe with
 * `useResources` so they redraw when it changes.
 */
import type { LucideIcon } from "lucide-react";
import { createElement, useCallback, useEffect, useState, useSyncExternalStore, type CSSProperties } from "react";

import { ICONS } from "./icons.js";
import { querySql } from "./api.js";
import { recordOpError } from "./opErrors.js";
import { NO_READS, useRerunOnChange } from "./viz/rerun.js";
import { identityOf, resourceUri, typeOf } from "./uri/uri.js";
import type { Reads, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";



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



const SVG_ICON_PREFIX = "data:image/svg+xml;base64,";
const svgIcons = new Map<string, LucideIcon>();

/** An svg an extension brought (published as a `data:` URI), as an icon
 *  component that sizes like the built-in ones. An `<img>` runs no script. */
function svgIcon(src: string): LucideIcon {
  let icon = svgIcons.get(src);
  if (!icon) {
    icon = (({ size = 16, className, style }: { size?: number | string; className?: string; style?: CSSProperties }) =>
      createElement("img", { src, alt: "", width: size, height: size, className, style })) as unknown as LucideIcon;
    svgIcons.set(src, icon);
  }
  return icon;
}

/** The icon of a resource, or null: a name oxplow draws, or the svg its
 *  extension brought. */
export function resourceIcon(resource: string): LucideIcon | null {
  const icon = current.get(resource)?.icon;
  if (!icon) return null;
  return icon.startsWith(SVG_ICON_PREFIX) ? svgIcon(icon) : (ICONS[icon] ?? null);
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
 * (`effort:eff3`, `wiki:x`). Null when it names none, or the value doesn't
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

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The resources; a component that draws or opens URIs reads this so it
 *  redraws when they change. */
export function useResources(): ReadonlyMap<string, Resource> {
  return useSyncExternalStore(
    subscribe,
    () => current,
    () => current,
  );
}

/** Every resource, and what was read. */
export async function readResources(): Promise<{ resources: Resource[]; reads: Reads }> {
  const res = await querySql(
    `SELECT resource, extension, label, params, additional, resolve, title, searchable, icon, wikilink
       FROM v_resource ORDER BY resource`,
    [],
    1_000,
  );
  return { resources: resourcesFromResult(res), reads: res.reads };
}

/** Load the list now and again whenever `v_resource` changes. Mounted
 *  once, in `App`. */
export function useResourcesLoader(): void {
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    void readResources()
      .then((r) => {
        setResources(r.resources);
        setReads(r.reads);
      })
      .catch((e: unknown) => {
        recordOpError({ label: "Read resources", message: e instanceof Error ? e.message : String(e) });
      });
  }, []);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
}

/** The title of an extension's ref from its kind's `resolve` model
 *  (`SELECT title … WHERE ref = ?`); null while loading, or when the
 *  model has no row for it. */
export function useExtensionRefTitle(ref: string | null): string | null {
  const resources = useResources();
  const resource = ref ? resources.get(typeOf(ref) ?? "") : undefined;
  const [title, setTitle] = useState<string | null>(null);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const view = resource?.resolve ?? null;
  const column = resource?.title ?? "title";
  const load = useCallback(() => {
    // View and column names are oxplow's (`v_<extension>_<model>`, a
    // model's column); never text a person typed. Checked anyway: they're
    // spliced into the query.
    if (!ref || !view || !/^v_[a-z0-9_]+$/.test(view) || !/^[a-z][a-z0-9_]*$/.test(column)) {
      setTitle(null);
      return;
    }
    // URI-SWITCH: the model's key column becomes `uri`.
    // A model's `uri` is the identity: the ref's view (`#part=…`) is the page's.
    void querySql(`SELECT ${column} FROM ${view} WHERE uri = ?1 LIMIT 1`, [identityOf(ref) ?? ref], 1)
      .then((res) => {
        const cell = res.rows[0]?.[0];
        setTitle(cell == null ? null : String(cell));
        setReads(res.reads);
      })
      .catch(() => setTitle(null));
  }, [ref, view, column]);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
  return title;
}
