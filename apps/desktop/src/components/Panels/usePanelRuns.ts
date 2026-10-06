/// The extension panels' lens runs (P6.G1): each panel's body, its badge
/// (the badge lens's alert count while it fires), its collapsed summary
/// and its count lens (tsk1089) — each distinct lens once. One owner (the
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
  /** The collapsed lens's run — the one-line summary a collapsed panel
   *  shows — when the panel has one. */
  collapsed: LensRun | null;
  /** The header's count (`panelCount`); null shows none. */
  count: number | null;
}

export function badgeCount(badge: LensRun | null): number | null {
  return badge?.alert?.firing ? badge.alert.count : null;
}

/** The header's count: the count lens's row count (a `number` lens: its
 *  value), which raises no alert; else the badge's count while it fires.
 *  A panel with both shows the count lens's; its badge still feeds Alerts. */
export function panelCount(count: LensRun | null, badge: LensRun | null): number | null {
  if (!count) return badgeCount(badge);
  if (count.lens.viz === "number") {
    const v = count.result.rows[0]?.[0] ?? null;
    const n = typeof v === "number" ? v : typeof v === "string" && v.trim() !== "" ? Number(v) : NaN;
    return Number.isFinite(n) ? n : null;
  }
  return count.result.rows.length;
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
        // Each distinct lens once, whatever roles it plays.
        const ids = [...new Set([p.body, p.badge, p.collapsed, p.count].filter((id): id is string => !!id))];
        const ran = new Map(
          await Promise.all(ids.map(async (id) => [id, await runLens(id, params, streamId).catch(() => null)] as const)),
        );
        const of = (id: string | null) => (id ? (ran.get(id) ?? null) : null);
        const badge = of(p.badge);
        const runs: PanelRuns = { body: of(p.body), badge, collapsed: of(p.collapsed), count: panelCount(of(p.count), badge) };
        return [p.id, runs, [...ran.values()]] as const;
      }),
    );
    setRuns(Object.fromEntries(entries.map(([id, r]) => [id, r])));
    setReads(unionReads(entries.flatMap(([, , ran]) => ran.map((r) => r?.result.reads))));
  }, [panels, streamId, threadId]);
  useEffect(() => void refresh(), [refresh]);
  useRerunOnChange(reads, () => void refresh());
  return runs;
}
