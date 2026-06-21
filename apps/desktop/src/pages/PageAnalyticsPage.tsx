import { useEffect, useState } from "react";

import {
  type CountByDayRowApi,
  type TopVisitedRowApi,
  countPageVisitsByDay,
  subscribePageVisitEvents,
  topVisitedPages,
} from "../api.js";
import { Card } from "../components/Card.js";
import { DailyBarChart } from "../components/Analytics/DailyBarChart.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";

const WINDOW_DAYS = 30;

/**
 * Page Analytics — the visit analytics that used to sit on the Go To
 * page: total visits, the most-visited pages (with counts), and a
 * visits-per-day trend chart. Linked from the Usage hub.
 */
export function PageAnalyticsPage({ onOpenPage }: { onOpenPage(ref: TabRef): void }) {
  const [top, setTop] = useState<TopVisitedRowApi[]>([]);
  const [byDay, setByDay] = useState<CountByDayRowApi[]>([]);

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      const since = new Date(Date.now() - WINDOW_DAYS * 24 * 60 * 60 * 1000).toISOString();
      void topVisitedPages({ sinceT: since, limit: 25 }).then((rows) => {
        if (!cancelled) setTop(rows);
      });
      void countPageVisitsByDay({ sinceT: since }).then((rows) => {
        if (!cancelled) setByDay(rows);
      });
    };
    refresh();
    const off = subscribePageVisitEvents(refresh);
    return () => {
      cancelled = true;
      off();
    };
  }, []);

  const totalVisits = byDay.reduce((sum, r) => sum + r.count, 0);

  return (
    <Page testId="page-page-analytics" title="Page Analytics">
      <div style={{ padding: "16px 20px", display: "flex", flexDirection: "column", gap: 20, maxWidth: 960 }}>
        <Card testId="page-analytics-by-day" title={`Visits per Day (Last ${WINDOW_DAYS}d)`}>
          <div style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)", marginBottom: 8 }}>
            {totalVisits} total visit{totalVisits === 1 ? "" : "s"}.
          </div>
          <DailyBarChart
            rows={byDay.map((r) => ({ label: r.day, value: r.count }))}
            emptyHint={`No visits in the last ${WINDOW_DAYS} days.`}
          />
        </Card>

        <Card testId="page-analytics-most-visited" title={`Most Visited (Last ${WINDOW_DAYS}d)`}>
          {top.length === 0 ? (
            <div style={{ color: "var(--text-muted)", fontSize: "var(--text-sm)", fontStyle: "italic" }}>
              No visits recorded yet.
            </div>
          ) : (
            <div style={{ display: "flex", flexDirection: "column" }}>
              {top.map((r) => (
                <button
                  key={r.refId}
                  type="button"
                  data-testid={`page-analytics-row-${r.refId}`}
                  onClick={() => onOpenPage({ id: r.refId, kind: r.refKind as TabRef["kind"], payload: r.payload })}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 8,
                    padding: "6px 4px",
                    background: "transparent",
                    border: "none",
                    borderBottom: "1px solid var(--border-subtle)",
                    cursor: "pointer",
                    textAlign: "left",
                    fontSize: "var(--text-sm)",
                    color: "var(--text-primary)",
                  }}
                >
                  <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {r.label}
                  </span>
                  <span style={{ color: "var(--text-muted)", fontSize: 11 }}>{r.refKind}</span>
                  <span
                    style={{
                      fontSize: 11,
                      color: "var(--text-secondary)",
                      background: "var(--surface-tab-inactive)",
                      padding: "1px 6px",
                      borderRadius: 999,
                    }}
                  >
                    {r.count}
                  </span>
                </button>
              ))}
            </div>
          )}
        </Card>
      </div>
    </Page>
  );
}
