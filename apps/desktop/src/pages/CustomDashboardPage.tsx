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
  subscribeOxplowEvents,
} from "../api.js";
import { MetricTile } from "../components/Dashboard/MetricTile.js";
import { InlineConfirm } from "../components/InlineConfirm.js";
import { InlineEdit } from "../components/InlineEdit.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { useContextMenu } from "../components/useRowContextMenu.js";
import { dashboardsRef } from "../tabs/pageRefs.js";
import { Page, pageH1Style } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import { buildAddMetricMenu } from "./customDashboardData.js";

/**
 * Custom dashboard — a project-global grid of metric tiles the user assembles
 * (tsk141, epic tsk138). Details layout: the dashboard title is an editable
 * in-body H1 (`titleInBody`), the rail carries the add/delete actions, and the
 * body is a responsive flow grid of {@link MetricTile}s. Tiles are added by the
 * rail button or a right-click "Add metric" menu (metrics grouped by category).
 * Live-refreshes on `dashboardsChanged` (structure) — tiles refresh their own
 * data on `metricSamplesChanged`.
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
  const addMenu = useContextMenu();

  usePageTitle(data?.dashboard.title ?? "Dashboard");

  useEffect(() => {
    if (!dashboardId) {
      setLoading(false);
      return;
    }
    let cancelled = false;
    const refresh = () => {
      void getDashboard(dashboardId).then((d) => {
        if (!cancelled) {
          setData(d);
          setLoading(false);
        }
      });
    };
    refresh();
    void listMetricDefinitions().then((rows) => {
      if (!cancelled) setDefs(new Map(rows.map((d) => [d.key, d])));
    });
    void listMetricCatalog().then((rows) => {
      if (!cancelled) setCatalog(rows);
    });
    // dashboardsChanged: our own or an agent's create/add/remove; configChanged:
    // a metric was enabled/disabled (defs list changes).
    const off = subscribeOxplowEvents((e) => {
      if (e.kind === "dashboardsChanged") refresh();
      if (e.kind === "configChanged") {
        void listMetricDefinitions().then((rows) => {
          if (!cancelled) setDefs(new Map(rows.map((d) => [d.key, d])));
        });
      }
    });
    return () => {
      cancelled = true;
      off();
    };
  }, [dashboardId]);

  const addTile = useCallback(
    async (metricKey: string) => {
      if (!dashboardId) return;
      try {
        await addDashboardItem({
          dashboardId,
          kind: "metric",
          metricKey,
          optionsJson: JSON.stringify({ viz: "line" }),
        });
      } catch (e) {
        recordOpError({ label: "Add tile", message: e instanceof Error ? e.message : String(e) });
      }
    },
    [dashboardId],
  );

  const removeTile = useCallback(async (itemId: string) => {
    try {
      await removeDashboardItem(itemId);
    } catch (e) {
      recordOpError({ label: "Remove tile", message: e instanceof Error ? e.message : String(e) });
    }
  }, []);

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

  const addMetricItems = useMemo(() => buildAddMetricMenu(catalog, (key) => void addTile(key)), [catalog, addTile]);

  const items = data?.dashboard ? data.items : [];

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
        style={{ padding: 24, display: "flex", flexDirection: "column", gap: 20 }}
        data-testid="custom-dashboard"
        onContextMenu={(e) => addMenu.open(e, addMetricItems)}
      >
        {/* Header row: the editable title on the left, the page-level actions
            on the right. A tile grid wants the full width, so this page is
            `layout="full"` and owns its own header rather than using the
            details rail. */}
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
              onClick={(e) => addMenu.open(e, addMetricItems)}
              data-testid="dashboard-add-metric"
              style={{
                fontSize: 13,
                padding: "6px 12px",
                borderRadius: 6,
                border: "1px solid var(--border-subtle)",
                background: "var(--surface-card)",
                color: "var(--text, #ddd)",
                cursor: "pointer",
              }}
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
                  style={{
                    fontSize: 13,
                    padding: "6px 12px",
                    borderRadius: 6,
                    border: "1px solid var(--border-subtle)",
                    background: "transparent",
                    color: "var(--danger, #f85149)",
                    cursor: "pointer",
                  }}
                >
                  Delete
                </button>
              )}
            </InlineConfirm>
          </div>
        </div>
        {items.length === 0 ? (
          <div
            data-testid="dashboard-empty"
            style={{
              border: "1px dashed var(--border-subtle)",
              borderRadius: 6,
              padding: 32,
              textAlign: "center",
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              gap: 12,
            }}
          >
            <div style={{ opacity: 0.7 }}>No tiles yet. Add a metric to get started.</div>
            <div style={{ fontSize: 12, opacity: 0.5 }}>Right-click anywhere here, or use “+ Add metric”.</div>
            <button
              type="button"
              onClick={(e) => addMenu.open(e, addMetricItems)}
              style={{
                fontSize: 13,
                padding: "6px 12px",
                borderRadius: 6,
                border: "1px solid var(--border-subtle)",
                background: "var(--surface-card)",
                color: "var(--text, #ddd)",
                cursor: "pointer",
              }}
            >
              + Add metric
            </button>
          </div>
        ) : (
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fill, minmax(320px, 1fr))",
              gap: 16,
            }}
            data-testid="dashboard-grid"
          >
            {items.map((it) => (
              <MetricTile
                key={it.id}
                item={it}
                def={it.metric_key ? (defs.get(it.metric_key) ?? null) : null}
                onOpenPage={onOpenPage}
                onRemove={() => void removeTile(it.id)}
              />
            ))}
          </div>
        )}
        {addMenu.menu}
      </div>
    );
  })();

  return (
    // `layout="full"`: a tile grid wants every pixel, not the details
    // layout's 78ch reading column + 320px rail. The page owns its own
    // padding + header row (title + actions) instead.
    <Page testId="page-custom-dashboard" title={data?.dashboard.title ?? "Dashboard"} titleInBody>
      {body}
    </Page>
  );
}
