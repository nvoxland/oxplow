import { useEffect, useState } from "react";

import {
  type AgentKindTokenUsage,
  type TokenUsageTotals,
  type TopVisitedRowApi,
  countPageVisitsByDay,
  getTokenTotalsOverall,
  subscribeOxplowEvents,
  subscribePageVisitEvents,
  tokenUsageByAgent,
  topVisitedPages,
} from "../api.js";
import { Card, cardLinkButton } from "../components/Card.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { indexRef } from "../tabs/pageRefs.js";
import { formatTokens } from "../tokens.js";

const WINDOW_DAYS = 30;

/**
 * Usage hub — a high-level dashboard over the two analytics areas:
 * **Page Analytics** (where you navigate) and **Token Analytics**
 * (model + agent/harness token spend). Each card summarizes and links
 * to its in-depth page. Read-only; no new IPC beyond the summaries.
 */
export function UsagePage({ onOpenPage }: { onOpenPage(ref: TabRef): void }) {
  const [totalVisits, setTotalVisits] = useState(0);
  const [topVisited, setTopVisited] = useState<TopVisitedRowApi[]>([]);
  const [tokens, setTokens] = useState<TokenUsageTotals | null>(null);
  const [topAgent, setTopAgent] = useState<AgentKindTokenUsage | null>(null);

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      const since = new Date(Date.now() - WINDOW_DAYS * 24 * 60 * 60 * 1000).toISOString();
      void countPageVisitsByDay({ sinceT: since }).then((rows) => {
        if (!cancelled) setTotalVisits(rows.reduce((sum, r) => sum + r.count, 0));
      });
      void topVisitedPages({ sinceT: since, limit: 3 }).then((rows) => {
        if (!cancelled) setTopVisited(rows);
      });
      void getTokenTotalsOverall().then((t) => {
        if (!cancelled) setTokens(t);
      });
      void tokenUsageByAgent().then((rows) => {
        if (!cancelled) setTopAgent(rows[0] ?? null);
      });
    };
    refresh();
    const offVisits = subscribePageVisitEvents(refresh);
    const offTokens = subscribeOxplowEvents((e) => {
      if (e.kind === "agentTokenUsageChanged") refresh();
    });
    return () => {
      cancelled = true;
      offVisits();
      offTokens();
    };
  }, []);

  return (
    <Page testId="page-usage" title="Usage">
      <div style={{ padding: "16px 20px", display: "flex", flexDirection: "column", gap: 20, maxWidth: 960 }}>
        <Card
          testId="usage-page-analytics-card"
          title="Page Analytics"
          action={
            <button
              type="button"
              data-testid="usage-open-page-analytics"
              style={cardLinkButton}
              onClick={() => onOpenPage(indexRef("page-analytics"))}
            >
              View Page Analytics →
            </button>
          }
        >
          <div style={{ fontSize: "var(--text-sm)", color: "var(--text-secondary)" }}>
            {totalVisits} page visit{totalVisits === 1 ? "" : "s"} in the last {WINDOW_DAYS} days.
          </div>
          {topVisited.length > 0 ? (
            <ul style={{ margin: "8px 0 0", paddingLeft: 18, color: "var(--text-secondary)", fontSize: "var(--text-xs)" }}>
              {topVisited.map((r) => (
                <li key={r.refId}>
                  {r.label} <span style={{ color: "var(--text-muted)" }}>· {r.count}</span>
                </li>
              ))}
            </ul>
          ) : null}
        </Card>

        <Card
          testId="usage-token-analytics-card"
          title="Token Analytics"
          action={
            <button
              type="button"
              data-testid="usage-open-token-analytics"
              style={cardLinkButton}
              onClick={() => onOpenPage(indexRef("token-analytics"))}
            >
              View Token Analytics →
            </button>
          }
        >
          <div style={{ fontSize: "var(--text-sm)", color: "var(--text-secondary)" }}>
            {formatTokens(tokens?.total_tokens ?? 0)} tokens across {tokens?.turns ?? 0} turn
            {tokens?.turns === 1 ? "" : "s"}.
          </div>
          <div style={{ marginTop: 4, fontSize: "var(--text-xs)", color: "var(--text-muted)", display: "flex", gap: 12 }}>
            <span>in {formatTokens(tokens?.input_tokens ?? 0)}</span>
            <span>out {formatTokens(tokens?.output_tokens ?? 0)}</span>
            <span>cache-w {formatTokens(tokens?.cache_creation_input_tokens ?? 0)}</span>
            <span>cache-r {formatTokens(tokens?.cache_read_input_tokens ?? 0)}</span>
          </div>
          {topAgent ? (
            <div style={{ marginTop: 6, fontSize: "var(--text-xs)", color: "var(--text-secondary)" }}>
              Busiest harness: <strong>{topAgent.agent_kind}</strong> ({formatTokens(topAgent.totals.total_tokens)})
            </div>
          ) : null}
        </Card>
      </div>
    </Page>
  );
}
