import { useCallback, useEffect, useState } from "react";
import { querySql, type DashboardItem, type LensRun, type LensViz } from "../../api.js";
import type { LensChart } from "../../tauri-bridge/generated/bindings.js";
import { LensResultView } from "../../lens/LensResultView.js";
import { adHocLens } from "../../lens/lensModel.js";
import { NO_READS, useRerunOnChange } from "../../lens/lensRerun.js";
import type { MenuItem } from "../../menu.js";
import type { TileOptions } from "../../pages/customDashboardData.js";
import { useRequestGuard } from "../../request-guard.js";
import type { TabRef } from "../../tabs/tabState.js";
import { useContextMenu } from "../useRowContextMenu.js";

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

/**
 * A `query` dashboard tile (P4.7): its title over a `QueryTileBody`. The
 * metric card display is `MetricTile`. See `.context/dashboards.md`.
 */
export function QueryTile({
  item,
  opts,
  onOpenPage,
  onRemove,
  onConfigure,
}: {
  item: DashboardItem;
  opts: TileOptions;
  onOpenPage?: (ref: TabRef) => void;
  onRemove?: () => void;
  onConfigure?: (next: Partial<TileOptions>) => void;
}) {
  const sql = opts.sql ?? "";
  const ctxMenu = useContextMenu();

  const menuItems: MenuItem[] = [
    {
      id: "size",
      label: "Size",
      enabled: !!onConfigure,
      submenu: (["small", "wide", "tall", "full"] as const).map((s) => ({
        id: `size:${s}`,
        label: s === "small" ? "Small" : s === "wide" ? "Wide" : s === "tall" ? "Tall" : "Full width",
        enabled: true,
        checked: (opts.size ?? "small") === s,
        run: () => onConfigure?.({ size: s }),
      })),
    },
    { id: "sep", label: "", enabled: false, separator: true },
    { id: "remove", label: "Remove from dashboard", enabled: !!onRemove, run: () => onRemove?.() },
  ];

  return (
    <section
      data-testid={`query-tile-${item.id}`}
      onContextMenu={(e) => ctxMenu.open(e, menuItems)}
      style={{
        background: "var(--surface-card)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 6,
        padding: 12,
        display: "flex",
        flexDirection: "column",
        gap: 8,
        minWidth: 0,
        height: "100%",
        overflow: "hidden",
      }}
    >
      <div style={{ fontWeight: 600, color: "var(--text-primary)" }} title={sql}>
        {opts.title ?? "Query"}
      </div>
      <QueryTileBody sql={sql} display={opts.display ?? null} chart={opts.chart ?? null} onOpenPage={onOpenPage} />
      {ctxMenu.menu}
    </section>
  );
}
