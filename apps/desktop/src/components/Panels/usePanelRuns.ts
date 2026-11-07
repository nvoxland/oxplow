/// The extension panels' page runs (P6.G1): each panel's body, its badge
/// (the badge page's alert count while it fires), its collapsed summary
/// and its count page — one open page run per part. One owner (the rail)
/// opens them all and hands each panel its runs; the Alerts panel is
/// derived from the same runs (`panelAlerts`), so a badge never runs twice
/// and the header count and Alerts can't disagree. Live: the daemon
/// re-runs them when what they read changes; the stream or thread the rail
/// shows changing opens new ones. What a part runs with is the daemon's
/// (`open_panel_part`, `crates/oxplow-app/src/pages/panels.rs`): the scope's
/// id from the stream and thread the rail shows, the body's toggles from the
/// viewer's picks, and each part's run says what it gives the panel
/// (`PartView`: whether the header counts by it, the count, the body's
/// toggles). The window reads those; it decides none of them.
import { useEffect, useMemo, useState } from "react";

import { windowPageRuns, type PageRuns } from "../../pageRuns.js";
import { pageFileUri } from "../../pages/pageFiles.js";
import type { Extension, ExtensionPanel, PageRun, PanelChoice, PartView } from "../../tauri-bridge/generated/bindings.js";

export type { PanelChoice };

/** A part's open page run: what its actions are pressed on and what it drew. */
export interface OpenPanelRun {
  runId: string | null;
  run: PageRun;
  /** What this run gives the panel, as the daemon says it. */
  view: PartView | null;
}

/** A panel's runs: one open page run per part, kept fresh by the daemon. */
export interface PanelRuns {
  body: OpenPanelRun | null;
  /** The one-line summary a collapsed panel shows, when it has one. */
  collapsed: OpenPanelRun | null;
  /** The badge page's run, when the panel has one. */
  badge: PageRun | null;
  /** The header's count, from the part the daemon says it counts by;
   *  null shows none. */
  count: number | null;
  /** The body's toggles, each with the value in force (the one it ran with). */
  choices: PanelChoice[];
}

/** The viewer's picks for the panels' choice params: by panel
 *  id, each param's chosen value. */
export type PanelChoices = Record<string, Record<string, string>>;

/** No picks — one value, so it doesn't re-run the panels each render. */
export const NO_CHOICES: PanelChoices = {};

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
 *  open run per part, which the daemon binds (`open_panel_part`), the body
 *  with the viewer's picks. The daemon re-runs them when what they read
 *  changes. */
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
          panel: { panel: p.id, part, choices: part === "body" ? (choices[p.id] ?? {}) : {} },
          params: {},
          streamId,
          threadId,
        });
        const take = () => {
          const { runId, run, panel: view } = handle.get();
          setState((s) => ({ ...s, [p.id]: { ...s[p.id], [part]: run ? { runId, run, view } : null } }));
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
      const counting = Object.values(s).find((r) => r?.view?.counts);
      out[p.id] = {
        body: s.body ?? null,
        collapsed: s.collapsed ?? null,
        badge: s.badge?.run ?? null,
        count: counting?.view?.count ?? null,
        choices: s.body?.view?.choices ?? [],
      };
    }
    return out;
  }, [panels, state]);
}
