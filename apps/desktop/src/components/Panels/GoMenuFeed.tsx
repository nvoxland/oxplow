/// The Go menu's lines (`menuBar.ts` `goMenuItems`): what the Go To pane
/// shows — the `go-to` panel's body run, as its toggle has it — handed up to
/// the window that builds the menu bar. Draws nothing.
import { useEffect } from "react";

import { goRowsOf, type GoRow } from "../../menuBar.js";
import { usePanelRuns } from "./PanelRunsContext.js";

/** The Go To panel's id (`<namespace>/<id>`). */
export const GO_TO_PANEL = "oxplow/go-to";

export function GoMenuFeed({ onRows }: { onRows(rows: GoRow[]): void }) {
  const body = usePanelRuns().runs[GO_TO_PANEL]?.body?.run ?? null;
  const key = JSON.stringify(goRowsOf(body));
  useEffect(() => {
    onRows(JSON.parse(key) as GoRow[]);
  }, [key, onRows]);
  return null;
}
