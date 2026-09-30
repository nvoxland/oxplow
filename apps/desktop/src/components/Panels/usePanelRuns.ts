/// An extension panel's lens runs (P6.G1): its body, and its badge — the
/// badge lens's alert count while it fires. Live: re-run when what they
/// read changes. The run binds the scope's `stream_id` / `thread_id` from
/// the current stream (and its selected thread) on the backend.
import { useCallback, useEffect, useState } from "react";

import { runLens, type LensRun } from "../../api.js";
import { NO_READS, unionReads, useRerunOnChange } from "../../lens/lensRerun.js";
import type { ExtensionPanel, Reads } from "../../tauri-bridge/generated/bindings.js";

export interface PanelRuns {
  body: LensRun | null;
  /** The badge lens's run, when the panel has one. */
  badge: LensRun | null;
  /** The badge's count while its alert fires; null otherwise. */
  count: number | null;
}

export function badgeCount(badge: LensRun | null): number | null {
  return badge?.alert?.firing ? badge.alert.count : null;
}

export function usePanelRuns(panel: ExtensionPanel, streamId: string | null, runBody = true): PanelRuns {
  const [body, setBody] = useState<LensRun | null>(null);
  const [badge, setBadge] = useState<LensRun | null>(null);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const refresh = useCallback(async () => {
    const [b, g] = await Promise.all([
      runBody ? runLens(panel.body, {}, streamId).catch(() => null) : Promise.resolve(null),
      panel.badge ? runLens(panel.badge, {}, streamId).catch(() => null) : Promise.resolve(null),
    ]);
    setBody(b);
    setBadge(g);
    setReads(unionReads([b?.result.reads, g?.result.reads]));
  }, [panel.body, panel.badge, streamId, runBody]);
  useEffect(() => void refresh(), [refresh]);
  useRerunOnChange(reads, () => void refresh());
  return { body, badge, count: badgeCount(badge) };
}
