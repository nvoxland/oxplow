/**
 * Every resource the vocabulary knows (`.context/resources.md`), read
 * from `v_resource`: oxplow's and those extensions declare. How the chrome
 * draws one (an extension's icon and label), how a `[[…]]` names one, and
 * which dataset holds it. What opens one is the page that renders it
 * (`pages/renderers.ts`).
 *
 * One process-wide list, loaded by `useResourcesLoader` (mounted once, in
 * `App`) and re-read when `v_resource` changes; the pure helpers
 * (`pageKindIconComponent`, `refFromTabId`, `preprocessWikilinks`) read
 * the current list, and components that render refs of extensions' kinds subscribe with
 * `useResources` so they redraw when it changes.
 */
import type { LucideIcon } from "lucide-react";
import { useCallback, useEffect, useState, useSyncExternalStore } from "react";

import { iconFor } from "./icons.js";
import { querySql } from "./api.js";
import { recordOpError } from "./opErrors.js";
import { NO_READS, useRerunOnChange } from "./viz/rerun.js";
import { typeOf } from "./uri/uri.js";
import type { Reads } from "./tauri-bridge/generated/bindings.js";
import { currentResources, extensionTitleQuery, resourcesFromResult, setResources, subscribe, type Resource } from "./resourceStore.js";



/** The icon of a resource, or null: a name oxplow draws, or the svg its
 *  extension brought. */
export function resourceIcon(resource: string): LucideIcon | null {
  const icon = currentResources().get(resource)?.icon;
  if (!icon) return null;
  return iconFor(icon);
}

/** The resources; a component that draws or opens URIs reads this so it
 *  redraws when they change. */
export function useResources(): ReadonlyMap<string, Resource> {
  return useSyncExternalStore(
    subscribe,
    currentResources,
    currentResources,
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

/** The title of an extension's ref from its kind's `resolve` dataset
 *  (`SELECT title … WHERE ref = ?`); null while loading, or when the
 *  dataset has no row for it. */
/** The same as `useExtensionRefTitle`, awaited (a component frame's
 *  markdown, drawn once its titles are known). */
export async function loadExtensionRefTitle(ref: string): Promise<string | null> {
  const query = extensionTitleQuery(ref, currentResources().get(typeOf(ref) ?? "") ?? undefined);
  if (!query) return null;
  try {
    const cell = (await querySql(query.sql, query.params, 1)).rows[0]?.[0];
    return cell == null ? null : String(cell);
  } catch {
    return null;
  }
}

export function useExtensionRefTitle(ref: string | null): string | null {
  const resources = useResources();
  const resource = ref ? resources.get(typeOf(ref) ?? "") : undefined;
  const [title, setTitle] = useState<string | null>(null);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    const query = ref ? extensionTitleQuery(ref, resource) : null;
    if (!query) {
      setTitle(null);
      return;
    }
    void querySql(query.sql, query.params, 1)
      .then((res) => {
        const cell = res.rows[0]?.[0];
        setTitle(cell == null ? null : String(cell));
        setReads(res.reads);
      })
      .catch(() => setTitle(null));
  }, [ref, resource]);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
  return title;
}
