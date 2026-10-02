import { EmptyState } from "../components/Prompts/EmptyState.js";
import { useCallback, useEffect, useMemo, useState } from "react";

import {
  type DashboardWithItems,
  type MetricCatalogEntry,
  type MetricSpec,
  addDashboardItem,
  deleteDashboard,
  getDashboard,
  listMetricCatalog,
  listMetricDefinitions,
  removeDashboardItem,
  renameDashboard,
  reorderDashboardItems,
  subscribeDashboardEvents,
  updateDashboardItem,
} from "../api.js";
import { NO_READS, unionReads, useRerunOnChange } from "../lens/lensRerun.js";
import { useRequestGuard } from "../request-guard.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";
import { MetricPickerPanel } from "../components/Dashboard/MetricPickerPanel.js";
import { MetricTile } from "../components/Dashboard/MetricTile.js";
import { TextTile } from "../components/Dashboard/TextTile.js";
import { LensTile } from "../components/Dashboard/LensTile.js";
import { QueryTile } from "../components/Dashboard/QueryTile.js";
import { moveToIndex } from "../components/CenterTabs/centerTabsReorder.js";
import { InlineConfirm } from "../components/InlineConfirm.js";
import { InlineEdit } from "../components/InlineEdit.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { dashboardsRef } from "../tabs/pageRefs.js";
import { Page, pageH1Style } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import { RANGE_PRESETS, rangeFromPreset } from "./metricDetailData.js";
import { type TileOptions, metricTile, parseTileOptions, tileSpanStyle } from "./customDashboardData.js";

/** Drag payload for tile reordering — distinct from the rail's section MIME so
 *  a rail drag can never drop into the grid. */
const TILE_MIME = "application/x-oxplow-dashboard-tile";

const selectStyle: React.CSSProperties = {
  fontSize: 12,
  background: "var(--surface-card)",
  color: "var(--text, #ddd)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: "4px 6px",
};

const buttonStyle: React.CSSProperties = {
  fontSize: 13,
  padding: "6px 12px",
  borderRadius: 6,
  border: "1px solid var(--border-subtle)",
  background: "var(--surface-card)",
  color: "var(--text, #ddd)",
  cursor: "pointer",
};

/**
 * Custom dashboard — a project-global grid of metric tiles the user assembles
 * (tsk141/tsk142, epic tsk138).
 *
 * `Page layout="full"`: a tile grid wants every pixel, so this page owns its
 * padding and renders its own header (editable title + actions) rather than
 * using the details layout's reading column + rail. Under the header sits the
 * **dashboard filter** (time range + branch) that every tile inherits unless it
 * overrides it. Tiles flow in a responsive grid, reorder by drag-and-drop, and
 * are added via the header button, the empty state, or a right-click menu.
 * Live-refreshes when `v_dashboard` / `v_dashboard_item` change (structure); the metric specs and
 * catalog re-read when their models change.
 */
export function CustomDashboardPage({
  dashboardId,
  onOpenPage,
}: {
  dashboardId?: string;
  onOpenPage?: (ref: TabRef, opts?: { newTab?: boolean }) => void;
}) {
  const [data, setData] = useState<DashboardWithItems | null>(null);
  const [loading, setLoading] = useState(true);
  const [defs, setDefs] = useState<Map<string, MetricSpec>>(new Map());
  const [catalog, setCatalog] = useState<MetricCatalogEntry[]>([]);
  // Dashboard-level filter, inherited by every tile.
  const [rangeKey, setRangeKey] = useState("all");
  const [branch, setBranch] = useState<string | null>(null);
  const [branches, setBranches] = useState<string[]>([]);
  // Drag-reorder state: the tile being dragged + the slot it would land in.
  const [dragId, setDragId] = useState<string | null>(null);
  const [overIndex, setOverIndex] = useState<number | null>(null);
  // Which slot the drop would land in, and on which side of that tile — so the
  // insertion line can sit in the gap the tile will actually move to.
  const [overSide, setOverSide] = useState<"before" | "after">("before");
  // The add-metric picker's anchor, or null when closed.
  const [pickerAt, setPickerAt] = useState<{ x: number; y: number } | null>(null);

  usePageTitle(data?.dashboard.title ?? "Dashboard");

  useEffect(() => {
    if (!dashboardId) {
      setLoading(false);
      return;
    }
    let cancelled = false;
    const refresh = () => {
      void getDashboard(dashboardId).then((d) => {
        if (cancelled) return;
        setData(d);
        setLoading(false);
      });
    };
    refresh();
    const off = subscribeDashboardEvents(refresh);
    return () => {
      cancelled = true;
      off();
    };
  }, [dashboardId]);

  // The metric specs (each metric tile's title, unit, direction) and the
  // catalog (the add-metric picker): re-read when either model changes.
  const [metricReads, setMetricReads] = useState<Reads>(NO_READS);
  const metricGuard = useRequestGuard();
  const loadMetrics = useCallback(() => {
    const current = metricGuard.begin();
    void Promise.all([listMetricDefinitions(), listMetricCatalog()]).then(([specs, entries]) => {
      if (!current()) return;
      setDefs(new Map(specs.rows.map((d) => [d.key, d])));
      setCatalog(entries.rows);
      setMetricReads(unionReads([specs.reads, entries.reads]));
    });
  }, [metricGuard]);
  useEffect(() => {
    loadMetrics();
  }, [loadMetrics]);
  useRerunOnChange(metricReads, loadMetrics);

  // Tiles report the branches present in their samples; the filter offers the
  // union. Returns the previous array when nothing is new so the identity stays
  // stable and tiles don't re-render in a loop.
  const mergeBranches = useCallback((incoming: string[]) => {
    setBranches((prev) => {
      const merged = new Set(prev);
      let grew = false;
      for (const b of incoming) {
        if (merged.has(b)) continue;
        merged.add(b);
        grew = true;
      }
      return grew ? [...merged].sort() : prev;
    });
  }, []);

  const dashboardFilter = useMemo(
    () => ({ range: rangeKey === "all" ? null : rangeFromPreset(rangeKey, Date.now()), branch }),
    [rangeKey, branch],
  );

  const addTile = useCallback(
    async (metricKey: string) => {
      if (!dashboardId) return;
      try {
        await addDashboardItem({ dashboardId, ...metricTile(metricKey) });
      } catch (e) {
        recordOpError({ label: "Add tile", message: e instanceof Error ? e.message : String(e) });
      }
    },
    [dashboardId],
  );

  const addTextTile = useCallback(async () => {
    if (!dashboardId) return;
    try {
      await addDashboardItem({
        dashboardId,
        kind: "text",
        // Seeded EMPTY on purpose: the tile's own "click to add a heading"
        // placeholder invites the real title. Pre-filling "## Section" implied
        // the band contained the tiles under it, which it doesn't (tsk147).
        optionsJson: JSON.stringify({ text: "", size: "full" }),
      });
    } catch (e) {
      recordOpError({ label: "Add text tile", message: e instanceof Error ? e.message : String(e) });
    }
  }, [dashboardId]);

  const removeTile = useCallback(async (itemId: string) => {
    try {
      await removeDashboardItem(itemId);
    } catch (e) {
      recordOpError({ label: "Remove tile", message: e instanceof Error ? e.message : String(e) });
    }
  }, []);

  /** Merge a partial option change into the tile's existing blob and persist. */
  const configureTile = useCallback(
    (itemId: string, current: TileOptions, next: Partial<TileOptions>) => {
      void updateDashboardItem(itemId, JSON.stringify({ ...current, ...next })).catch((e) =>
        recordOpError({ label: "Configure tile", message: e instanceof Error ? e.message : String(e) }),
      );
    },
    [],
  );

  const rename = useCallback(
    (next: string) => {
      if (!dashboardId) return;
      void renameDashboard(dashboardId, next).catch((e) =>
        recordOpError({ label: "Rename dashboard", message: e instanceof Error ? e.message : String(e) }),
      );
    },
    [dashboardId],
  );

  const removeDashboard = useCallback(() => {
    if (!dashboardId) return;
    void deleteDashboard(dashboardId)
      .then(() => onOpenPage?.(dashboardsRef()))
      .catch((e) =>
        recordOpError({ label: "Delete dashboard", message: e instanceof Error ? e.message : String(e) }),
      );
  }, [dashboardId, onOpenPage]);

  const items = data?.dashboard ? data.items : [];
  // Only draw the insertion line while a tile drag is actually in flight.
  const showLine = dragId !== null;

  /** Drop the dragged tile into slot `index` and persist the new order. */
  const dropAt = useCallback(
    (index: number) => {
      const ids = items.map((i) => i.id);
      const id = dragId;
      setDragId(null);
      setOverIndex(null);
      if (!id || !dashboardId) return;
      const next = moveToIndex(ids, id, index);
      if (next.join("\u0000") === ids.join("\u0000")) return;
      void reorderDashboardItems(dashboardId, next).catch((e) =>
        recordOpError({ label: "Reorder tiles", message: e instanceof Error ? e.message : String(e) }),
      );
    },
    [dashboardId, dragId, items],
  );

  // Metric keys already on this dashboard — the picker marks them ✓ so a
  // second pass doesn't silently duplicate a tile.
  const addedKeys = useMemo(
    () =>
      new Set(
        items
          .map((i) => (i.kind === "query" ? parseTileOptions(i.options_json).metric : undefined))
          .filter((k): k is string => !!k),
      ),
    [items],
  );

  const body = (() => {
    if (loading) return <div style={{ padding: 24, opacity: 0.6 }}>Loading…</div>;
    if (!data?.dashboard)
      return (
        <div style={{ padding: 24, opacity: 0.6 }} data-testid="dashboard-missing">
          This dashboard no longer exists.
        </div>
      );
    return (
      <div
        style={{ padding: 24, display: "flex", flexDirection: "column", gap: 16 }}
        data-testid="custom-dashboard"
        onContextMenu={(e) => {
          e.preventDefault();
          setPickerAt({ x: e.clientX, y: e.clientY });
        }}
      >
        {/* Header row: the editable title on the left, page-level actions right. */}
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 16 }}>
          <h1 style={{ margin: 0, minWidth: 0 }}>
            <InlineEdit
              value={data.dashboard.title}
              onCommit={rename}
              displayStyle={pageH1Style}
              ariaLabel="Dashboard title"
              testId="dashboard-title"
            />
          </h1>
          <div style={{ display: "flex", alignItems: "center", gap: 8, flexShrink: 0 }}>
            <button
              type="button"
              onClick={(e) => {
                const r = e.currentTarget.getBoundingClientRect();
                // Anchor under the button, right-aligned to it.
                setPickerAt({ x: r.right - 340, y: r.bottom + 4 });
              }}
              data-testid="dashboard-add-metric"
              style={buttonStyle}
            >
              + Add metric
            </button>
            <InlineConfirm onConfirm={removeDashboard} confirmLabel="Delete" testIdPrefix="dashboard-delete">
              {(arm) => (
                <button
                  type="button"
                  onClick={arm}
                  data-testid="dashboard-delete"
                  title="Delete this dashboard"
                  style={{ ...buttonStyle, background: "transparent", color: "var(--danger, #f85149)" }}
                >
                  Delete
                </button>
              )}
            </InlineConfirm>
          </div>
        </div>

        {/* Dashboard filter — every tile inherits this unless it overrides. */}
        <div style={{ display: "flex", alignItems: "center", gap: 10, fontSize: 12, opacity: 0.85 }}>
          <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <span style={{ opacity: 0.6 }}>Range</span>
            <select
              value={rangeKey}
              onChange={(e) => setRangeKey(e.target.value)}
              data-testid="dashboard-range"
              style={selectStyle}
            >
              <option value="all">All time</option>
              {RANGE_PRESETS.map((p) => (
                <option key={p.key} value={p.key}>
                  {p.label}
                </option>
              ))}
            </select>
          </label>
          <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <span style={{ opacity: 0.6 }}>Branch</span>
            <select
              value={branch ?? ""}
              onChange={(e) => setBranch(e.target.value || null)}
              data-testid="dashboard-branch"
              style={selectStyle}
            >
              <option value="">All branches</option>
              {branches.map((b) => (
                <option key={b} value={b}>
                  {b}
                </option>
              ))}
            </select>
          </label>
        </div>

        {items.length === 0 ? (
          <div style={{ border: "1px dashed var(--border-subtle)", borderRadius: 6, padding: "12px 24px" }}>
            <EmptyState
              testId="dashboard-empty"
              title="No tiles yet"
              text="Right-click anywhere here, or use + Add metric; the agent can pin a chart or a lens too."
              prompts={["Pin a chart of this project's test coverage over time to this dashboard"]}
            >
              <div>
                <button
                  type="button"
                  onClick={(e) => {
                    const r = e.currentTarget.getBoundingClientRect();
                    setPickerAt({ x: r.left, y: r.bottom + 4 });
                  }}
                  style={buttonStyle}
                >
                  + Add metric
                </button>
              </div>
            </EmptyState>
          </div>
        ) : (
          <div
            style={{
              display: "grid",
              // 400px min: a line tile has to fit a labeled time axis, and at
              // ~320px the chart scaled down until its ticks were unreadable
              // (tsk144).
              gridTemplateColumns: "repeat(auto-fill, minmax(400px, 1fr))",
              // Rows size to content: a heading band must be able to be one
              // line tall. Metric tiles carry their own min-height instead, so
              // charts still get room (tsk147).
              gridAutoRows: "auto",
              gap: 16,
            }}
            data-testid="dashboard-grid"
            onDragOver={(e) => {
              if (!e.dataTransfer.types.includes(TILE_MIME)) return;
              e.preventDefault();
              // Only fires when the pointer is over the grid's own background —
              // a tile's handler stops propagation. Show the drop landing after
              // the last tile, matching what the background drop does.
              setOverIndex(items.length - 1);
              setOverSide("after");
            }}
            onDrop={(e) => {
              // Dropped on the grid background → move to the end.
              if (!e.dataTransfer.types.includes(TILE_MIME)) return;
              e.preventDefault();
              dropAt(items.length);
            }}
          >
            {items.map((it, index) => {
              const opts = parseTileOptions(it.options_json);
              const isDragging = dragId === it.id;
              return (
                <div
                  key={it.id}
                  draggable
                  onDragStart={(e) => {
                    e.dataTransfer.setData(TILE_MIME, it.id);
                    e.dataTransfer.effectAllowed = "move";
                    setDragId(it.id);
                  }}
                  onDragEnd={() => {
                    setDragId(null);
                    setOverIndex(null);
                  }}
                  onDragOver={(e) => {
                    if (!e.dataTransfer.types.includes(TILE_MIME)) return;
                    e.preventDefault();
                    e.stopPropagation();
                    // Which half of the tile the pointer is over decides
                    // whether the drop lands before or after it.
                    const r = e.currentTarget.getBoundingClientRect();
                    setOverIndex(index);
                    setOverSide(e.clientX < r.left + r.width / 2 ? "before" : "after");
                  }}
                  onDrop={(e) => {
                    if (!e.dataTransfer.types.includes(TILE_MIME)) return;
                    e.preventDefault();
                    e.stopPropagation();
                    // Recompute the side from THIS event rather than reading
                    // `overSide` state: the final dragOver's setState may not
                    // have re-rendered before drop fires, which would drop on
                    // the stale side.
                    const r = e.currentTarget.getBoundingClientRect();
                    const after = e.clientX >= r.left + r.width / 2;
                    dropAt(after ? index + 1 : index);
                  }}
                  data-testid={`dashboard-tile-slot-${it.id}`}
                  style={{
                    // A text tile is a heading band: full grid width unless it
                    // was explicitly sized otherwise.
                    ...tileSpanStyle(it.kind === "text" ? (opts.size ?? "full") : opts.size),
                    minWidth: 0,
                    opacity: isDragging ? 0.4 : 1,
                    // Anchors the absolutely-positioned insertion line below.
                    position: "relative",
                    borderRadius: 6,
                  }}
                >
                  {/* Insertion line — an absolutely positioned bar sitting in
                      the grid gap. It must live OUTSIDE the tile card: an
                      inset box-shadow on this wrapper (the first attempt) is
                      painted under the opaque card and never shows. */}
                  {showLine && overIndex === index && !isDragging ? (
                    <div
                      data-testid="dashboard-drop-line"
                      style={{
                        position: "absolute",
                        top: 0,
                        bottom: 0,
                        [overSide === "before" ? "left" : "right"]: -9,
                        width: 3,
                        borderRadius: 2,
                        background: "var(--accent, #58a6ff)",
                        pointerEvents: "none",
                        zIndex: 2,
                      }}
                    />
                  ) : null}
                  {it.kind === "lens" ? (
                    <LensTile
                      item={it}
                      opts={opts}
                      onOpenPage={onOpenPage ? (ref) => onOpenPage(ref) : undefined}
                      onRemove={() => void removeTile(it.id)}
                      onConfigure={(next) => configureTile(it.id, opts, next)}
                    />
                  ) : it.kind === "text" ? (
                    <TextTile
                      item={it}
                      opts={opts}
                      onRemove={() => void removeTile(it.id)}
                      onConfigure={(next) => configureTile(it.id, opts, next)}
                    />
                  ) : opts.display !== "metric" ? (
                    <QueryTile
                      item={it}
                      opts={opts}
                      onOpenPage={onOpenPage ? (ref) => onOpenPage(ref) : undefined}
                      onRemove={() => void removeTile(it.id)}
                      onConfigure={(next) => configureTile(it.id, opts, next)}
                    />
                  ) : (
                    <MetricTile
                      item={it}
                      opts={opts}
                      def={opts.metric ? (defs.get(opts.metric) ?? null) : null}
                      dashboard={dashboardFilter}
                      onOpenPage={onOpenPage}
                      onRemove={() => void removeTile(it.id)}
                      onConfigure={(next) => configureTile(it.id, opts, next)}
                      onBranches={mergeBranches}
                    />
                  )}
                </div>
              );
            })}
          </div>
        )}
        {pickerAt ? (
          <MetricPickerPanel
            catalog={catalog}
            anchor={pickerAt}
            addedKeys={addedKeys}
            // Adding leaves the panel open so several tiles land in one pass.
            onPick={(key) => void addTile(key)}
            onAddText={() => {
              void addTextTile();
              setPickerAt(null);
            }}
            onClose={() => setPickerAt(null)}
          />
        ) : null}
      </div>
    );
  })();

  return (
    <Page testId="page-custom-dashboard" title={data?.dashboard.title ?? "Dashboard"} titleInBody>
      {body}
    </Page>
  );
}
