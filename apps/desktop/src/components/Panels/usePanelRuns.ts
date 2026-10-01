/// The extension panels' lens runs (P6.G1): each panel's body, and its
/// badge — the badge lens's alert count while it fires. One owner (the
/// rail) runs them all, once per refresh, and hands each panel its runs;
/// the Alerts panel is derived from the same runs (`panelAlerts`), so a
/// badge never runs twice and the header count and Alerts can't disagree.
/// Live: re-run when what they read changes, and when the stream or
/// thread the rail shows changes. The nav states each scope's binding
/// (`panelParams`) rather than leaving the backend to infer the thread.
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

export interface PanelAlert {
  /** The badge lens (`<extension>/<slug>`); opening the alert opens it. */
  id: string;
  title: string;
  message: string;
}

/** The Alerts panel's rows: every panel whose badge fires, from the runs
 *  the rail already made. */
export function panelAlerts(panels: readonly ExtensionPanel[], runs: Readonly<Record<string, PanelRuns>>): PanelAlert[] {
  return panels.flatMap((p) => {
    const badge = p.badge ? runs[p.id]?.badge : null;
    return badge?.alert?.firing ? [{ id: badge.lens.id, title: badge.lens.title, message: badge.alert.message }] : [];
  });
}

/** Every panel's runs, by panel id, for the stream and thread shown. */
export function useExtensionPanelRuns(
  panels: readonly ExtensionPanel[],
  streamId: string | null,
  threadId: string | null,
): Record<string, PanelRuns> {
  const [runs, setRuns] = useState<Record<string, PanelRuns>>({});
  const [reads, setReads] = useState<Reads>(NO_READS);
  // `panels` is the rail's loaded list (state), so its identity changes
  // only when the panels do.
  const refresh = useCallback(async () => {
    const entries = await Promise.all(
      panels.map(async (p) => {
        const params = panelParams(p.scope, streamId, threadId);
        const [body, badge] = await Promise.all([
          runLens(p.body, params, streamId).catch(() => null),
          p.badge ? runLens(p.badge, params, streamId).catch(() => null) : Promise.resolve(null),
        ]);
        return [p.id, { body, badge, count: badgeCount(badge) }] as const;
      }),
    );
    setRuns(Object.fromEntries(entries));
    setReads(unionReads(entries.flatMap(([, r]) => [r.body?.result.reads, r.badge?.result.reads])));
  }, [panels, streamId, threadId]);
  useEffect(() => void refresh(), [refresh]);
  useRerunOnChange(reads, () => void refresh());
  return runs;
}
