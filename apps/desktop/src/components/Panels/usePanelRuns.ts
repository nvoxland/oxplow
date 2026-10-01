/// An extension panel's lens runs (P6.G1): its body, and its badge — the
/// badge lens's alert count while it fires. Live: re-run when what they
/// read changes, and when the stream or thread the panel is shown for
/// changes. The nav states the scope's binding (`panelParams`) rather
/// than leaving the backend to infer the thread from the selection.
import { useCallback, useEffect, useState } from "react";

import { runLens, type LensRun } from "../../api.js";
import { NO_READS, unionReads, useRerunOnChange } from "../../lens/lensRerun.js";
import { streamRowId, threadRowId } from "../../modelIds.js";
import type { ExtensionPanel, PanelScope, Reads, SqlCell } from "../../tauri-bridge/generated/bindings.js";

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

/** What a panel's lenses are bound to, by its scope: the stream's row id
 *  as `stream_id`, the thread's as `thread_id`, nothing for a project
 *  panel. Without a thread (or stream) to bind, a scoped panel binds
 *  nothing and its lens falls back to its default. */
export function panelParams(
  scope: PanelScope,
  streamId: string | null,
  threadId: string | null,
): Record<string, SqlCell> {
  switch (scope) {
    case "project":
      return {};
    case "stream":
      return streamId ? { stream_id: streamRowId(streamId) } : {};
    case "thread":
      return threadId ? { thread_id: threadRowId(threadId) } : {};
  }
}

export function usePanelRuns(
  panel: ExtensionPanel,
  streamId: string | null,
  threadId: string | null,
  runBody = true,
): PanelRuns {
  const [body, setBody] = useState<LensRun | null>(null);
  const [badge, setBadge] = useState<LensRun | null>(null);
  const [reads, setReads] = useState<Reads>(NO_READS);
  // Keyed on the panel's fields, not its identity: a caller may build the
  // panel object per render.
  const { body: bodyLens, badge: badgeLens, scope } = panel;
  const refresh = useCallback(async () => {
    const params = panelParams(scope, streamId, threadId);
    const [b, g] = await Promise.all([
      runBody ? runLens(bodyLens, params, streamId).catch(() => null) : Promise.resolve(null),
      badgeLens ? runLens(badgeLens, params, streamId).catch(() => null) : Promise.resolve(null),
    ]);
    setBody(b);
    setBadge(g);
    setReads(unionReads([b?.result.reads, g?.result.reads]));
  }, [bodyLens, badgeLens, scope, streamId, threadId, runBody]);
  useEffect(() => void refresh(), [refresh]);
  useRerunOnChange(reads, () => void refresh());
  return { body, badge, count: badgeCount(badge) };
}
