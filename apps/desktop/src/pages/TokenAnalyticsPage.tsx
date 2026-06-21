import { useEffect, useMemo, useState } from "react";

import {
  type AgentKindTokenUsage,
  type ModelTokenUsage,
  type TokenUsageByDay,
  type TokenUsageTotals,
  getTokenTotalsOverall,
  subscribeOxplowEvents,
  tokenUsageByAgent,
  tokenUsageByDay,
  tokenUsageByModel,
} from "../api.js";
import { Card } from "../components/Card.js";
import { DailyBarChart } from "../components/Analytics/DailyBarChart.js";
import { Page } from "../tabs/Page.js";
import { formatTokens } from "../tokens.js";

const WINDOW_DAYS = 30;

/**
 * Token Analytics — model + agent/harness token usage across every
 * recorded turn: overall totals (in/out/cache), a by-harness rollup, a
 * by-model breakdown (models nested under their harness), and a
 * tokens-per-day trend. Linked from the Usage hub. Live-refreshes on
 * `agentTokenUsageChanged`. (Cost is out of scope — no pricing table yet.)
 */
export function TokenAnalyticsPage() {
  const [overall, setOverall] = useState<TokenUsageTotals | null>(null);
  const [byAgent, setByAgent] = useState<AgentKindTokenUsage[]>([]);
  const [byModel, setByModel] = useState<ModelTokenUsage[]>([]);
  const [byDay, setByDay] = useState<TokenUsageByDay[]>([]);

  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      void getTokenTotalsOverall().then((t) => {
        if (!cancelled) setOverall(t);
      });
      void tokenUsageByAgent().then((rows) => {
        if (!cancelled) setByAgent(rows);
      });
      void tokenUsageByModel().then((rows) => {
        if (!cancelled) setByModel(rows);
      });
      void tokenUsageByDay(WINDOW_DAYS).then((rows) => {
        if (!cancelled) setByDay(rows);
      });
    };
    refresh();
    const off = subscribeOxplowEvents((e) => {
      if (e.kind === "agentTokenUsageChanged") refresh();
    });
    return () => {
      cancelled = true;
      off();
    };
  }, []);

  // Group the by-model rows under their harness for a nested display.
  const modelsByAgent = useMemo(() => {
    const map = new Map<string, ModelTokenUsage[]>();
    for (const row of byModel) {
      const list = map.get(row.agent_kind) ?? [];
      list.push(row);
      map.set(row.agent_kind, list);
    }
    return [...map.entries()];
  }, [byModel]);

  const empty = (overall?.turns ?? 0) === 0;

  return (
    <Page testId="page-token-analytics" title="Token Analytics">
      <div style={{ padding: "16px 20px", display: "flex", flexDirection: "column", gap: 20, maxWidth: 960 }}>
        {empty ? (
          <div style={{ color: "var(--text-muted)", fontSize: "var(--text-sm)", fontStyle: "italic" }}>
            No token usage recorded yet — usage is captured from agent transcripts on each Stop.
          </div>
        ) : null}

        <Card testId="token-analytics-overall" title="Overall">
          <div style={{ fontSize: "var(--text-base)", fontWeight: 600, color: "var(--text-primary)" }}>
            {formatTokens(overall?.total_tokens ?? 0)} tokens
            <span style={{ fontWeight: 400, color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>
              {" "}· {overall?.turns ?? 0} turn{overall?.turns === 1 ? "" : "s"}
            </span>
          </div>
          <div className="oxplow-tabular" style={{ marginTop: 6, display: "flex", gap: 14, fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>
            <span title="Fresh input tokens">in {formatTokens(overall?.input_tokens ?? 0)}</span>
            <span title="Output tokens">out {formatTokens(overall?.output_tokens ?? 0)}</span>
            <span title="Cache-write (creation) tokens">cache-w {formatTokens(overall?.cache_creation_input_tokens ?? 0)}</span>
            <span title="Cache-read tokens">cache-r {formatTokens(overall?.cache_read_input_tokens ?? 0)}</span>
          </div>
        </Card>

        <Card testId="token-analytics-by-agent" title="By Agent / Harness">
          {byAgent.length === 0 ? (
            <Empty />
          ) : (
            <div style={{ display: "flex", flexDirection: "column" }}>
              {byAgent.map((row) => (
                <UsageRow
                  key={row.agent_kind}
                  testId={`token-analytics-agent-${row.agent_kind}`}
                  label={row.agent_kind}
                  totals={row.totals}
                />
              ))}
            </div>
          )}
        </Card>

        <Card testId="token-analytics-by-model" title="By Model">
          {modelsByAgent.length === 0 ? (
            <Empty />
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
              {modelsByAgent.map(([agentKind, models]) => (
                <div key={agentKind}>
                  <div style={{ fontSize: 11, fontWeight: 600, color: "var(--text-secondary)", textTransform: "uppercase", letterSpacing: 0.4, marginBottom: 2 }}>
                    {agentKind}
                  </div>
                  {models.map((row) => (
                    <UsageRow
                      key={`${agentKind}:${row.model ?? "unknown"}`}
                      testId={`token-analytics-model-${agentKind}-${row.model ?? "unknown"}`}
                      label={row.model ?? "(unknown model)"}
                      mono
                      totals={row.totals}
                    />
                  ))}
                </div>
              ))}
            </div>
          )}
        </Card>

        <Card testId="token-analytics-by-day" title={`Tokens per Day (Last ${WINDOW_DAYS}d)`}>
          <DailyBarChart
            rows={byDay.map((r) => ({ label: r.day, value: r.total_tokens }))}
            emptyHint={`No token usage in the last ${WINDOW_DAYS} days.`}
            formatValue={formatTokens}
          />
        </Card>
      </div>
    </Page>
  );
}

function UsageRow({
  label,
  totals,
  testId,
  mono,
}: {
  label: string;
  totals: TokenUsageTotals;
  testId?: string;
  mono?: boolean;
}) {
  return (
    <div
      data-testid={testId}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "5px 4px",
        borderBottom: "1px solid var(--border-subtle)",
        fontSize: "var(--text-sm)",
      }}
    >
      <span
        style={{
          flex: 1,
          minWidth: 0,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
          color: "var(--text-primary)",
          fontFamily: mono ? "var(--font-mono)" : undefined,
        }}
      >
        {label}
      </span>
      <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
        {totals.turns} turn{totals.turns === 1 ? "" : "s"}
      </span>
      <span className="oxplow-tabular" style={{ color: "var(--text-secondary)", fontSize: "var(--text-xs)", minWidth: 80, textAlign: "right" }}>
        {formatTokens(totals.total_tokens)}
      </span>
    </div>
  );
}

function Empty() {
  return (
    <div style={{ color: "var(--text-muted)", fontSize: "var(--text-sm)", fontStyle: "italic" }}>
      No token usage recorded yet.
    </div>
  );
}
