/**
 * A page's stored change analysis (`v_change*`): ensures the change on
 * mount, asks again when its stream goes stale (working tree, open
 * effort), and refreshes when its analysis lands. Slots pass the returned
 * `changeId` to their lenses. See `.context/semantic-layer.md` →
 * "Change analysis".
 */
import { useEffect, useState } from "react";
import { ensureChange, subscribeOxplowEvents } from "../api.js";
import type { ChangeRow, ChangeTarget } from "../tauri-bridge/generated/bindings.js";

/** Whether `event` means the page should ask for its change again: its
 *  stream went stale (working tree / open effort), or its analysis landed. */
export function shouldReensure(
  event: { kind: string; streamId?: unknown; changeId?: unknown },
  target: ChangeTarget,
  row: Pick<ChangeRow, "id" | "streamId"> | null,
): boolean {
  if (!row) return false;
  if (event.kind === "changeAnalyzed") return event.changeId === row.id;
  if (event.kind === "changeStale") return target.kind !== "commit" && event.streamId === row.streamId;
  return false;
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
    const off = subscribeOxplowEvents((event) => {
      if (shouldReensure(event as unknown as { kind: string }, target, current)) ensure();
    });
    return () => {
      live = false;
      off();
    };
    // `key` stands in for `target` (a fresh object each render).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  return { change, error };
}
