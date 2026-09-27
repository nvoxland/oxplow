import { useCallback, useEffect, useState } from "react";
import { runLens, subscribeOxplowEvents, type DashboardItem, type LensRun } from "../../api.js";
import { LensResultView } from "../../lens/LensResultView.js";
import { shouldRerunLens } from "../../lens/lensModel.js";
import type { MenuItem } from "../../menu.js";
import type { TileOptions } from "../../pages/customDashboardData.js";
import { lensRef } from "../../tabs/pageRefs.js";
import { RouteLink } from "../../tabs/RouteLink.js";
import type { TabRef } from "../../tabs/tabState.js";
import { useContextMenu } from "../useRowContextMenu.js";

/** Rows a lens tile shows before "open the lens for the rest". */
const TILE_MAX_ROWS = 8;

/**
 * A `lens` dashboard tile: the lens's current result, compact. Dashboards
 * are project-global, so the lens runs against the primary stream. The
 * title links to the full lens page. See `.context/extensions.md`.
 */
export function LensTile({
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
  const lensId = opts.lensId ?? "";
  const [run, setRun] = useState<LensRun | null>(null);
  const [error, setError] = useState<string | null>(null);
  const ctxMenu = useContextMenu();

  const refresh = useCallback(async () => {
    if (!lensId) return;
    try {
      setRun(await runLens(lensId, {}, null));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [lensId]);

  useEffect(() => {
    void refresh();
    let timer: ReturnType<typeof setTimeout> | null = null;
    const off = subscribeOxplowEvents((event) => {
      if (!shouldRerunLens({ kind: event.kind, path: (event as { path?: unknown }).path })) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void refresh(), 750);
    });
    return () => {
      if (timer) clearTimeout(timer);
      off();
    };
  }, [refresh]);

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

  const open = (ref: TabRef) => onOpenPage?.(ref);
  const title = opts.title ?? run?.lens.title ?? lensId;

  return (
    <section
      data-testid={`lens-tile-${item.id}`}
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
      <RouteLink
        to={lensRef(lensId)}
        onNavigate={() => open(lensRef(lensId))}
        style={{ background: "none", border: "none", padding: 0, textAlign: "left", font: "inherit", fontWeight: 600, cursor: "pointer", color: "var(--text-primary)" }}
      >
        {title}
      </RouteLink>
      {error ? (
        <div style={{ fontSize: "var(--text-xs)", color: "var(--severity-critical)" }}>{error}</div>
      ) : run ? (
        <div style={{ overflow: "auto", minHeight: 0 }}>
          <LensResultView run={run} onOpenPage={open} maxRows={TILE_MAX_ROWS} />
        </div>
      ) : (
        <div style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)" }}>Loading…</div>
      )}
      {ctxMenu.menu}
    </section>
  );
}
