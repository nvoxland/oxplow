import { useCallback, useEffect, useState } from "react";
import { querySql, type LensRun, type LensViz } from "../../api.js";
import type { LensChart } from "../../tauri-bridge/generated/bindings.js";
import { LensResultView } from "../../lens/LensResultView.js";
import { adHocLens } from "../../lens/lensModel.js";
import { NO_READS, useRerunOnChange } from "../../lens/lensRerun.js";
import { useRequestGuard } from "../../request-guard.js";
import type { TabRef } from "../../tabs/tabState.js";

/** Rows a query tile shows. */
const TILE_MAX_ROWS = 8;

/**
 * A query tile's body: pinned SQL over the published models, shown as a
 * lens viz (`display`, `table` when absent), kept live by what it read.
 * The custom-dashboard page's `QueryTile` and a dashboard lens's row
 * tiles draw it.
 */
export function QueryTileBody({
  sql,
  display: shown,
  chart,
  maxRows = TILE_MAX_ROWS,
  onOpenPage,
}: {
  sql: string;
  display: string | null;
  chart: LensChart | null;
  maxRows?: number;
  onOpenPage?: (ref: TabRef) => void;
}) {
  const display = (shown ?? "table") as LensViz;
  const chartKey = JSON.stringify(chart);
  const [run, setRun] = useState<LensRun | null>(null);
  const [error, setError] = useState<string | null>(null);
  const guard = useRequestGuard();

  const refresh = useCallback(async () => {
    if (!sql) return;
    const current = guard.begin();
    try {
      const result = await querySql(sql, [], null);
      if (!current()) return;
      setRun({ lens: adHocLens(sql, display, JSON.parse(chartKey) as typeof chart), params: {}, result, alert: null, warnings: [], inactive: null });
      setError(null);
    } catch (e) {
      if (current()) setError(e instanceof Error ? e.message : String(e));
    }
  }, [sql, display, chartKey, guard]);

  useEffect(() => {
    void refresh();
  }, [refresh]);
  useRerunOnChange(run?.result.reads ?? NO_READS, () => void refresh());

  if (error) return <div style={{ fontSize: "var(--text-xs)", color: "var(--severity-critical)" }}>{error}</div>;
  if (!run) return <div style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)" }}>Loading…</div>;
  return (
    <div style={{ overflow: "auto", minHeight: 0 }}>
      <LensResultView run={run} onOpenPage={(ref) => onOpenPage?.(ref)} maxRows={maxRows} />
    </div>
  );
}
