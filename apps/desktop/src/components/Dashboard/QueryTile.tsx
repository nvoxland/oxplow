import { useCallback, useEffect, useState } from "react";
import { querySql, type DashboardItem, type LensRun, type LensViz } from "../../api.js";
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
 * A `query` dashboard tile (P4.7): pinned SQL over the published models,
 * shown as a lens viz (`display`), kept live by what it read. The metric
 * card display is `MetricTile`. See `.context/dashboards.md`.
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
  const display = (opts.display ?? "table") as LensViz;
  const chart = opts.chart ?? null;
  const chartKey = JSON.stringify(chart);
  const [run, setRun] = useState<LensRun | null>(null);
  const [error, setError] = useState<string | null>(null);
  const ctxMenu = useContextMenu();
  const guard = useRequestGuard();

  const refresh = useCallback(async () => {
    if (!sql) return;
    const current = guard.begin();
    try {
      const result = await querySql(sql, [], null);
      if (!current()) return;
      setRun({ lens: adHocLens(sql, display, JSON.parse(chartKey) as typeof chart), params: {}, result, alert: null });
      setError(null);
    } catch (e) {
      if (current()) setError(e instanceof Error ? e.message : String(e));
    }
  }, [sql, display, chartKey, guard]);

  useEffect(() => {
    void refresh();
  }, [refresh]);
  useRerunOnChange(run?.result.reads ?? NO_READS, () => void refresh());

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
      {error ? (
        <div style={{ fontSize: "var(--text-xs)", color: "var(--severity-critical)" }}>{error}</div>
      ) : run ? (
        <div style={{ overflow: "auto", minHeight: 0 }}>
          <LensResultView run={run} onOpenPage={(ref) => onOpenPage?.(ref)} maxRows={TILE_MAX_ROWS} />
        </div>
      ) : (
        <div style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)" }}>Loading…</div>
      )}
      {ctxMenu.menu}
    </section>
  );
}
