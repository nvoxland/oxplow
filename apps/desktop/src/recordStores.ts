/**
 * The stores of the shared models (`v_model_store`, `.context/models.md`),
 * as the app last read them (`recordStoresLoader.ts` keeps this current):
 * what makes a URI of any model's resource one of a store's records, and
 * what a `[[…]]` in markdown names — `[[<store>:<id>]]` for any store, and a
 * bare `[[id]]` for the first store (in the order read) whose `id_pattern`
 * matches it whole, the stores whose ids are `loose` (they count in running
 * text: `tsk42`) before a file or commit reading and the others after it, as
 * core's one record recognizer does. No store and no
 * resource is named here: a model an extension adds links and titles with
 * no change. Free of React and of the app's transport, so the markdown link
 * code (which the fixture harness bundles too) can read it.
 */
import { identityOf, parseUri, resourceUri, typeOf } from "./uri/uri.js";

export interface RecordStore {
  model: string;
  store: string;
  /** The model's resource: what its records' URIs are of. */
  resource: string;
  /** The param of those URIs that names the store (`store`, `provider`). */
  storeParam: string;
  /** The param that names the record in its store (`id`). */
  key: string;
  /** How its ids look in text, a regex matched whole; null: only `[[<store>:<id>]]` names its records. */
  idPattern: string | null;
  /** Its ids count in running text, not only inside `[[…]]`: they win over a path or a sha. */
  loose: boolean;
}

let stores: RecordStore[] = [];
const listeners = new Set<() => void>();

/** The stores now, in the order read. */
export function recordStores(): readonly RecordStore[] {
  return stores;
}

export function setRecordStores(next: readonly RecordStore[]): void {
  stores = [...next];
  for (const l of listeners) l();
}

/** Hear every change of the stores; the returned function stops. */
export function subscribeRecordStores(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The URI of `store`'s record `id`. */
export function recordUri(store: RecordStore, id: string): string {
  return resourceUri(store.resource, { [store.storeParam]: store.store, [store.key]: id });
}

/** The record `[[<store>:<id>]]` names: the URI, or null when no such store
 *  (the first of that id wins when two models share one). */
export function recordOfStoreId(store: string, id: string): string | null {
  const s = stores.find((x) => x.store === store);
  return s && id ? recordUri(s, id) : null;
}

/** The record a bare id names among the stores whose ids are `loose` (or
 *  those that aren't): the first whose id pattern matches it whole; null when
 *  none does (a store with no pattern says nothing of its ids). */
export function recordOfBareId(id: string, loose: boolean): string | null {
  for (const s of stores) {
    if (!s.idPattern || s.loose !== loose) continue;
    try {
      if (new RegExp(`^(?:${s.idPattern})$`).test(id)) return recordUri(s, id);
    } catch {
      // A pattern that isn't a regex says nothing.
    }
  }
  return null;
}

/** The record a bare id names whichever way its store counts it. */
export function recordOfAnyBareId(id: string): string | null {
  return recordOfBareId(id, true) ?? recordOfBareId(id, false);
}

/** The record `uri` names, or null when it isn't a URI of a store's records. */
export function recordOf(uri: string): { store: string; id: string; resource: string } | null {
  const resource = typeOf(uri);
  if (!resource) return null;
  const params = parseUri(uri)?.params ?? {};
  for (const s of stores) {
    if (s.resource !== resource) continue;
    const store = params[s.storeParam];
    const id = params[s.key];
    if (store === s.store && id) return { store, id, resource };
  }
  return null;
}

/** How a person names a record: its own id in its store (`tsk42`, `ENG-12`);
 *  the input when it isn't one. */
export function recordLabel(uri: string): string {
  return recordOf(uri)?.id ?? uri;
}

/** How a record is written in a `[[…]]`: by its id when a bare id names it
 *  (its store is the first that matches), else `store:id`. */
export function recordLinkText(uri: string): string {
  const rec = recordOf(uri);
  if (!rec) return uri;
  return recordOfAnyBareId(rec.id) === (identityOf(uri) ?? uri) ? rec.id : `${rec.store}:${rec.id}`;
}
