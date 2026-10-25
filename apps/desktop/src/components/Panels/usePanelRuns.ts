/// The extension panels' lens runs (P6.G1): each panel's body, its badge
/// (the badge lens's alert count while it fires), its collapsed summary
/// and its count lens — each distinct lens once. One owner (the
/// rail) runs them all, once per refresh, and hands each panel its runs;
/// the Alerts panel is derived from the same runs (`panelAlerts`), so a
/// badge never runs twice and the header count and Alerts can't disagree.
/// Live: re-run when what they read changes, and when the stream or
/// thread the rail shows changes. The nav states each scope's binding
/// (`panelParams`) rather than leaving the backend to infer the thread.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { runLens, type LensRun } from "../../api.js";
import { NO_READS, unionReads, useRerunOnChange } from "../../lens/lensRerun.js";
import { streamRowId, threadRowId } from "../../modelIds.js";
import { windowPageRuns, type PageRuns } from "../../pageRuns.js";
import { pageFileUri } from "../../pages/pageFiles.js";
import { lensRef } from "../../tabs/pageRefs.js";
import type { Extension, ExtensionPanel, LensParam, LensParamOption, PageRun, PanelScope, Reads, SqlCell } from "../../tauri-bridge/generated/bindings.js";

/** A part's open page run: what its actions are pressed on and what it drew. */
export interface OpenPanelRun {
  runId: string | null;
  run: PageRun;
}

/** A page-backed panel's (`pages: true`) runs: one open page run per part,
 *  kept fresh by the daemon. */
export interface PagePanelRuns {
  body: OpenPanelRun | null;
  collapsed: OpenPanelRun | null;
  badge: PageRun | null;
}

export interface PanelRuns {
  /** A page-backed panel's parts; its lens runs below are then null. */
  page?: PagePanelRuns;
  body: LensRun | null;
  /** The badge lens's run, when the panel has one. */
  badge: LensRun | null;
  /** The collapsed lens's run — the one-line summary a collapsed panel
   *  shows — when the panel has one. */
  collapsed: LensRun | null;
  /** The header's count (`panelCount`); null shows none. */
  count: number | null;
}

export function badgeCount(badge: LensRun | PageRun | null): number | null {
  return badge?.alert?.firing ? badge.alert.count : null;
}

/** A count page's number: its badge, else its first loader's row count. */
export function pageCount(count: PageRun | null): number | null {
  if (!count) return null;
  if (count.badge !== null && count.badge !== undefined) {
    const n = typeof count.badge === "number" ? count.badge : Number(count.badge);
    return Number.isFinite(n) ? n : null;
  }
  const first = count.page.loaders[0]?.name;
  const result = first ? count.loaders[first]?.result : undefined;
  return result ? result.rows.length : null;
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

/** The viewer's picks for the panels' choice params: by panel
 *  id, each param's chosen value. */
export type PanelChoices = Record<string, Record<string, string>>;

/** No picks — one value, so it doesn't re-run the panels each render. */
export const NO_CHOICES: PanelChoices = {};

/** One choice param of a panel's body lens, as its header toggle shows
 *  it: the options, and the value in force — the viewer's pick while the
 *  lens still offers it, else the default. */
export interface PanelChoice {
  name: string;
  label: string | null;
  options: LensParamOption[];
  value: string;
}

export function panelChoices(body: LensRun | PageRun | null, picked: Readonly<Record<string, string>>): PanelChoice[] {
  const params: LensParam[] = !body ? [] : "lens" in body ? body.lens.params : body.page.params;
  return params
    .filter((p) => p.options.length > 0)
    .map((p) => {
      const pick = picked[p.name];
      const value = pick != null && p.options.some((o) => o.value === pick) ? pick : String(p.default ?? p.options[0]!.value);
      return { name: p.name, label: p.label, options: p.options, value };
    });
}

export interface PanelAlert {
  /** The badge lens or page (`<extension>/<slug>`). */
  id: string;
  title: string;
  message: string;
  /** What opening the alert opens: the badge's lens or page (a URI). */
  open: string;
}

/** The Alerts panel's rows: every panel whose badge fires, from the runs
 *  the rail already made. */
export function panelAlerts(
  panels: readonly ExtensionPanel[],
  runs: Readonly<Record<string, PanelRuns>>,
  extensions: readonly Extension[] = [],
): PanelAlert[] {
  return panels.flatMap((p) => {
    if (!p.badge) return [];
    const page = runs[p.id]?.page?.badge;
    if (page) {
      return page.alert?.firing
        ? [{ id: page.page.id, title: page.page.title, message: page.alert.message, open: pageFileUri(extensions, page.page) }]
        : [];
    }
    const badge = runs[p.id]?.badge;
    return badge?.alert?.firing ? [{ id: badge.lens.id, title: badge.lens.title, message: badge.alert.message, open: lensRef(badge.lens.id).id }] : [];
  });
}

/** Every panel's runs, by panel id, for the stream and thread shown. */
export function useExtensionPanelRuns(
  panels: readonly ExtensionPanel[],
  streamId: string | null,
  threadId: string | null,
  choices: PanelChoices = NO_CHOICES,
  /** The window's page runs (default); a test's fake. */
  pageStore?: PageRuns,
): Record<string, PanelRuns> {
  const [runs, setRuns] = useState<Record<string, PanelRuns>>({});
  const [reads, setReads] = useState<Reads>(NO_READS);
  // Refreshes can overlap (a change arrives while one is in flight); only
  // the latest one started may set the state, or a slow older run that
  // began before the change would put its stale rows back.
  const latest = useRef(0);
  // `panels` is the rail's loaded list (state), so its identity changes
  // only when the panels do.
  const lensPanels = useMemo(() => panels.filter((p) => !p.pages), [panels]);
  const refresh = useCallback(async () => {
    const mine = ++latest.current;
    const entries = await Promise.all(
      lensPanels.map(async (p) => {
        const params = panelParams(p.scope, streamId, threadId);
        // The viewer's picks bind the body lens, which declares them.
        const bodyParams = { ...params, ...(choices[p.id] ?? {}) };
        // Each distinct lens once, whatever roles it plays.
        const ids = [...new Set([p.body, p.badge, p.collapsed, p.count].filter((id): id is string => !!id))];
        const ran = new Map(
          await Promise.all(
            ids.map(async (id) => [id, await runLens(id, id === p.body ? bodyParams : params, streamId).catch(() => null)] as const),
          ),
        );
        const of = (id: string | null) => (id ? (ran.get(id) ?? null) : null);
        const badge = of(p.badge);
        const runs: PanelRuns = { body: of(p.body), badge, collapsed: of(p.collapsed), count: panelCount(of(p.count), badge) };
        return [p.id, runs, [...ran.values()]] as const;
      }),
    );
    if (mine !== latest.current) return;
    setRuns(Object.fromEntries(entries.map(([id, r]) => [id, r])));
    setReads(unionReads(entries.flatMap(([, , ran]) => ran.map((r) => r?.result.reads))));
  }, [lensPanels, streamId, threadId, choices]);
  useEffect(() => void refresh(), [refresh]);
  useRerunOnChange(reads, () => void refresh());
  const pageRuns = usePagePanelRuns(panels, streamId, threadId, choices, pageStore);
  return useMemo(() => ({ ...runs, ...pageRuns }), [runs, pageRuns]);
}

type Part = "body" | "collapsed" | "badge" | "count";

/** The page-backed panels' runs, by panel id: one open page run per part
 *  (each distinct page once per panel), its params the panel's scope's
 *  (`stream_id` / `thread_id`) and, for the body, the viewer's picks. The
 *  daemon re-runs them when what they read changes. */
export function usePagePanelRuns(
  panels: readonly ExtensionPanel[],
  streamId: string | null,
  threadId: string | null,
  choices: PanelChoices = NO_CHOICES,
  store?: PageRuns,
): Record<string, PanelRuns> {
  const [state, setState] = useState<Record<string, Partial<Record<Part, OpenPanelRun | null>>>>({});
  const pagePanels = useMemo(() => panels.filter((p) => p.pages), [panels]);
  const choicesKey = JSON.stringify(choices);
  useEffect(() => {
    const runs = store ?? windowPageRuns();
    const offs: (() => void)[] = [];
    setState({});
    for (const p of pagePanels) {
      const params = panelParams(p.scope, streamId, threadId);
      const parts: [Part, string | null][] = [
        ["body", p.body],
        ["collapsed", p.collapsed],
        ["badge", p.badge],
        ["count", p.count],
      ];
      for (const [part, page] of parts) {
        if (!page) continue;
        const handle = runs.open({
          page,
          params: part === "body" ? { ...params, ...(choices[p.id] ?? {}) } : params,
          streamId,
          threadId,
        });
        const take = () => {
          const { runId, run } = handle.get();
          setState((s) => ({ ...s, [p.id]: { ...s[p.id], [part]: run ? { runId, run } : null } }));
        };
        take();
        const off = handle.subscribe(take);
        offs.push(() => {
          off();
          handle.close();
        });
      }
    }
    return () => offs.forEach((off) => off());
    // `choicesKey` stands in for `choices`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pagePanels, streamId, threadId, choicesKey, store]);
  return useMemo(() => {
    const out: Record<string, PanelRuns> = {};
    for (const p of pagePanels) {
      const s = state[p.id] ?? {};
      const badge = s.badge?.run ?? null;
      // The count page's number wins; else the badge's: an alert's count
      // while it fires, or the value a `badge:` declares.
      const count = p.count ? pageCount(s.count?.run ?? null) : badge?.alert ? badgeCount(badge) : badge?.badge != null ? pageCount(badge) : null;
      out[p.id] = { page: { body: s.body ?? null, collapsed: s.collapsed ?? null, badge }, body: null, badge: null, collapsed: null, count };
    }
    return out;
  }, [pagePanels, state]);
}
