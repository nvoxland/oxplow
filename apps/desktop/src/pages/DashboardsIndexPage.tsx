import { useEffect, useState } from "react";

import { type Dashboard, createDashboard, listDashboards, subscribeDashboardEvents } from "../api.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { customDashboardRef } from "../tabs/pageRefs.js";
import { Page } from "../tabs/Page.js";
import { RouteLink } from "../tabs/RouteLink.js";
import type { TabRef } from "../tabs/tabState.js";

/**
 * Dashboards index (tsk141, epic tsk138) — the list of the user's custom
 * dashboards, with a "New dashboard" action that creates-then-opens (the
 * NewStreamPage create→navigate pattern, no form). Each row navigates to the
 * dashboard's page; rows go through `RouteLink` so plain-click is in-tab and
 * modifier-click opens a new tab. Live-refreshes on `dashboardsChanged`.
 */
export function DashboardsIndexPage({
  onOpenPage,
}: {
  onOpenPage?: (ref: TabRef, opts?: { newTab?: boolean }) => void;
}) {
  const [dashboards, setDashboards] = useState<Dashboard[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void listDashboards().then((rows) => {
        if (!cancelled) {
          setDashboards(rows);
          setLoading(false);
        }
      });
    };
    refresh();
    const off = subscribeDashboardEvents(refresh);
    return () => {
      cancelled = true;
      off();
    };
  }, []);

  const newDashboard = () => {
    void createDashboard("Untitled dashboard")
      .then((d) => onOpenPage?.(customDashboardRef(d.id)))
      .catch((e) =>
        recordOpError({ label: "New dashboard", message: e instanceof Error ? e.message : String(e) }),
      );
  };

  return (
    <Page testId="page-dashboards" title="Dashboards">
      <div style={{ padding: 24, display: "flex", flexDirection: "column", gap: 16, maxWidth: 720 }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <h1 style={{ margin: 0, fontSize: 20, fontWeight: 600 }}>Dashboards</h1>
          <button
            type="button"
            onClick={newDashboard}
            data-testid="dashboards-new"
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
            + New dashboard
          </button>
        </div>
        {loading ? (
          <div style={{ opacity: 0.6 }}>Loading…</div>
        ) : dashboards.length === 0 ? (
          <div style={{ opacity: 0.6 }} data-testid="dashboards-empty">
            No dashboards yet. Create one to compose a grid of metric tiles.
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            {dashboards.map((d) => (
              <RouteLink
                key={d.id}
                to={customDashboardRef(d.id)}
                onNavigate={(ref, opts) => onOpenPage?.(ref, opts)}
                testId={`dashboard-row-${d.id}`}
                style={{
                  all: "unset",
                  cursor: "pointer",
                  padding: "10px 12px",
                  borderRadius: 6,
                  border: "1px solid var(--border-subtle)",
                  background: "var(--surface-card)",
                  fontSize: 14,
                  color: "var(--text, #ddd)",
                }}
              >
                {d.title}
              </RouteLink>
            ))}
          </div>
        )}
      </div>
    </Page>
  );
}
