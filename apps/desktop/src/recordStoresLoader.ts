import { useEffect, useState } from "react";
import { onRemoteReconnect, querySql, subscribeOxplowEvents } from "./api.js";
import { recordStores, setRecordStores, subscribeRecordStores, type RecordStore } from "./recordStores.js";
import type { SqlQueryResult, Reads } from "./tauri-bridge/generated/bindings.js";
import { coalesce, NO_READS, readsChanged } from "./viz/rerun.js";

/**
 * Loads the stores of every shared model (`v_model_store`) into
 * `recordStores.ts`: read once at boot, re-read when the rows it read
 * change (a store's extension turned on or off). Markdown reads them to
 * know what a `[[…]]` names; nothing here names a model or a store.
 */

/** The stores in a `v_model_store` read (`model, store, resource,
 *  store_param, key, id_pattern, loose`); a store whose model names no resource
 *  has no records to link to. */
export function recordStoresFromResult(result: SqlQueryResult): RecordStore[] {
  const col = (name: string) => result.columns.indexOf(name);
  const text = (row: SqlQueryResult["rows"][number], name: string): string | null => {
    const v = row[col(name)];
    return v === null || v === undefined || v === "" ? null : String(v);
  };
  // Only a folder of the project is a path a file list holds.
  const files = (row: SqlQueryResult["rows"][number]): RecordStore["files"] => {
    const folder = text(row, "files_folder");
    const suffix = text(row, "files_suffix");
    return folder && suffix && text(row, "files_root") === "project" ? { folder, suffix } : null;
  };
  const out: RecordStore[] = [];
  for (const row of result.rows) {
    const resource = text(row, "resource");
    const store = text(row, "store");
    if (!resource || !store) continue;
    out.push({
      model: text(row, "model") ?? "",
      store,
      resource,
      storeParam: text(row, "store_param") ?? "store",
      key: text(row, "key") ?? "id",
      idPattern: text(row, "id_pattern"),
      loose: Number(row[col("loose")] ?? 0) === 1 || row[col("loose")] === true,
      files: files(row),
    });
  }
  return out;
}

export async function readRecordStores(): Promise<{ stores: RecordStore[]; reads: Reads }> {
  const res = await querySql(
    "SELECT model, store, resource, store_param, key, id_pattern, loose, files_root, files_folder, files_suffix FROM v_model_store ORDER BY model, store",
    [],
    500,
  );
  return { stores: recordStoresFromResult(res), reads: res.reads };
}

let reads: Reads = NO_READS;
let started = false;
let first: Promise<void> | null = null;

async function refresh(): Promise<void> {
  try {
    const out = await readRecordStores();
    reads = out.reads;
    setRecordStores(out.stores);
  } catch {
    // Keep what we had; the next change re-reads.
  }
}

/** Start the shared read (the app does at boot, so markdown knows the
 *  stores before any surface that draws it mounts). */
export function startRecordStores(): void {
  if (started) return;
  started = true;
  first = refresh();
  const rerun = coalesce(() => void refresh());
  subscribeOxplowEvents((event) => {
    if (readsChanged(event as Record<string, unknown>, reads)) rerun.schedule();
  });
  onRemoteReconnect(rerun.schedule);
}

/** The stores once they have been read (a component frame's markdown). */
export async function loadRecordStores(): Promise<readonly RecordStore[]> {
  startRecordStores();
  await first;
  return recordStores();
}

/** The stores, re-rendering when they change. */
export function useRecordStores(): readonly RecordStore[] {
  const [value, setValue] = useState<readonly RecordStore[]>(recordStores);
  useEffect(() => {
    const update = () => setValue(recordStores());
    const stop = subscribeRecordStores(update);
    startRecordStores();
    update();
    return stop;
  }, []);
  return value;
}
