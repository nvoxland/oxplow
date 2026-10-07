/**
 * A page's stored change analysis (`v_change*`): ensures the change on
 * mount and asks again whenever `v_change` changes — the analysis landed,
 * or the `change.analyze` consumer recomputed a working tree or open
 * effort as its stream moved — once a burst of such commits goes quiet
 * (`coalesce`). Slots pass the returned `changeId` to their
 * lenses. See `.context/semantic-layer.md` → "Change analysis".
 */
import { useEffect, useState } from "react";
import { ensureChange, subscribeOxplowEvents } from "../api.js";
import { coalesce, readsChanged } from "./lensRerun.js";
import type { ChangeRow, ChangeTarget } from "../tauri-bridge/generated/bindings.js";

/** Whether `event` means the page should ask for its change again: a
 *  commit that touched `v_change` (its analysis landed or was recomputed). */
export function shouldReensure(event: Readonly<Record<string, unknown>>, row: Pick<ChangeRow, "id"> | null): boolean {
  return row !== null && readsChanged(event, { models: ["v_change"], tables: [], measures: [] });
}

export function useChange(target: ChangeTarget | null): { change: ChangeRow | null; error: string | null } {
  const [change, setChange] = useState<ChangeRow | null>(null);
  const [error, setError] = useState<string | null>(null);
  const key = target ? JSON.stringify(target) : null;
  useEffect(() => {
    if (!target) {
      setChange(null);
      return;
    }
    let live = true;
    let current: ChangeRow | null = null;
    const ensure = () =>
      void ensureChange(target)
        .then((row) => {
          current = row;
          if (live) {
            setChange(row);
            setError(null);
          }
        })
        .catch((e) => {
          if (live) setError(e instanceof Error ? e.message : String(e));
        });
    ensure();
    const again = coalesce(ensure);
    const off = subscribeOxplowEvents((event) => {
      if (shouldReensure(event, current)) again.schedule();
    });
    return () => {
      live = false;
      again.cancel();
      off();
    };
    // `key` stands in for `target` (a fresh object each render).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  return { change, error };
}
