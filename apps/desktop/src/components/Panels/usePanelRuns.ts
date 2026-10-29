/// The extension panels' page runs (P6.G1): each panel's body, its badge
/// (the badge page's alert count while it fires), its collapsed summary
/// and its count page — one open page run per part. One owner (the rail)
/// opens them all and hands each panel its runs; the Alerts panel is
/// derived from the same runs (`panelAlerts`), so a badge never runs twice
/// and the header count and Alerts can't disagree. Live: the daemon
/// re-runs them when what they read changes; the stream or thread the rail
/// shows changing opens new ones. The nav states each scope's binding
/// (`panelParams`) rather than leaving the backend to infer the thread.
import { useEffect, useMemo, useState } from "react";

import { streamRowId, threadRowId } from "../../modelIds.js";
import { windowPageRuns, type PageRuns } from "../../pageRuns.js";
import { firstLoader } from "../../pages/firstLoader.js";
import { pageFileUri } from "../../pages/pageFiles.js";
import type { Extension, ExtensionPanel, ChoiceOption, PageRun, PanelScope, SqlCell } from "../../tauri-bridge/generated/bindings.js";

/** A part's open page run: what its actions are pressed on and what it drew. */
export interface OpenPanelRun {
  runId: string | null;
  run: PageRun;
}

/** A panel's runs: one open page run per part, kept fresh by the daemon. */
export interface PanelRuns {
  body: OpenPanelRun | null;
  /** The one-line summary a collapsed panel shows, when it has one. */
  collapsed: OpenPanelRun | null;
  /** The badge page's run, when the panel has one. */
  badge: PageRun | null;
  /** The header's count: the count page's, else the badge's; null shows none. */
  count: number | null;
}

export function badgeCount(badge: PageRun | null): number | null {
  return badge?.alert?.firing ? badge.alert.count : null;
}

/** A count page's number: its badge, else its first loader's row count. */
export function pageCount(count: PageRun | null): number | null {
  if (!count) return null;
  if (count.badge !== null && count.badge !== undefined) {
    const n = typeof count.badge === "number" ? count.badge : Number(count.badge);
    return Number.isFinite(n) ? n : null;
  }
  const first = firstLoader(count.page)?.name;
  const result = first ? count.loaders[first]?.result : undefined;
  return result ? result.rows.length : null;
}

/** What a panel's pages are bound to, by its scope: the stream's row id
 *  as `stream_id`, the thread's as `thread_id`, nothing for a project
 *  panel. Without a thread (or stream) to bind, a scoped panel binds
 *  nothing and its page falls back to its default. */
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

/** One choice param of a panel's body page, as its header toggle shows
 *  it: the options, and the value in force — the viewer's pick while the
 *  page still offers it, else the default. */
export interface PanelChoice {
  name: string;
  label: string | null;
  options: ChoiceOption[];
  value: string;
}

export function panelChoices(body: PageRun | null, picked: Readonly<Record<string, string>>): PanelChoice[] {
  const params = body?.page.params ?? [];
  return params
    .filter((p) => p.options.length > 0)
    .map((p) => {
      const pick = picked[p.name];
      const value = pick != null && p.options.some((o) => o.value === pick) ? pick : String(p.default ?? p.options[0]!.value);
      return { name: p.name, label: p.label, options: p.options, value };
    });
}

export interface PanelAlert {
  /** The badge page (`<extension>/<slug>`). */
  id: string;
  title: string;
  message: string;
  /** What opening the alert opens: the badge's page (a URI). */
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
    const badge = runs[p.id]?.badge;
    return badge?.alert?.firing
      ? [{ id: badge.page.id, title: badge.page.title, message: badge.alert.message, open: pageFileUri(extensions, badge.page) }]
      : [];
  });
}

type Part = "body" | "collapsed" | "badge" | "count";

/** Every panel's runs, by panel id, for the stream and thread shown: one
 *  open page run per part, its params the panel's scope's (`stream_id` /
 *  `thread_id`) and, for the body, the viewer's picks. The daemon re-runs
 *  them when what they read changes. */
export function useExtensionPanelRuns(
  panels: readonly ExtensionPanel[],
  streamId: string | null,
  threadId: string | null,
  choices: PanelChoices = NO_CHOICES,
  store?: PageRuns,
): Record<string, PanelRuns> {
  const [state, setState] = useState<Record<string, Partial<Record<Part, OpenPanelRun | null>>>>({});
  const choicesKey = JSON.stringify(choices);
  useEffect(() => {
    const runs = store ?? windowPageRuns();
    const offs: (() => void)[] = [];
    setState({});
    for (const p of panels) {
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
  }, [panels, streamId, threadId, choicesKey, store]);
  return useMemo(() => {
    const out: Record<string, PanelRuns> = {};
    for (const p of panels) {
      const s = state[p.id] ?? {};
      const badge = s.badge?.run ?? null;
      // The count page's number wins; else the badge's: an alert's count
      // while it fires, or the value a `badge:` declares.
      const count = p.count ? pageCount(s.count?.run ?? null) : badge?.alert ? badgeCount(badge) : badge?.badge != null ? pageCount(badge) : null;
      out[p.id] = { body: s.body ?? null, collapsed: s.collapsed ?? null, badge, count };
    }
    return out;
  }, [panels, state]);
}
