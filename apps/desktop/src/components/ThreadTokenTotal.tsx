import { useEffect, useState } from "react";

import {
  type TokenUsageTotals,
  getThreadTokenTotals,
  subscribeOxplowEvents,
} from "../api.js";
import { formatTokens } from "../tokens.js";

/**
 * Running per-thread agent token total (tsk104) for the Work panel header.
 * Sums every turn's usage captured on Stop across the thread's efforts (and
 * any effort-less turns). Tokens-only for now. Self-hides until the thread
 * has at least one captured turn; live-updates on `agentTokenUsageChanged`.
 */
export function ThreadTokenTotal({ threadId }: { threadId: string }) {
  const [totals, setTotals] = useState<TokenUsageTotals | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      void getThreadTokenTotals(threadId).then((t) => {
        if (!cancelled) setTotals(t);
      });
    };
    load();
    const unsub = subscribeOxplowEvents((event) => {
      if (event.kind !== "agentTokenUsageChanged") return;
      if (event.threadId !== threadId) return;
      load();
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [threadId]);

  if (!totals || totals.turns === 0) return null;

  return (
    <div
      data-testid="thread-token-total"
      title={`${totals.input_tokens} input · ${totals.output_tokens} output · ${totals.cache_creation_input_tokens} cache-write · ${totals.cache_read_input_tokens} cache-read tokens across ${totals.turns} turns`}
      style={{
        display: "flex",
        alignItems: "baseline",
        gap: 6,
        padding: "4px 8px",
        fontSize: "var(--text-xs)",
        color: "var(--text-muted)",
        borderBottom: "1px solid var(--border-subtle)",
      }}
    >
      <span style={{ textTransform: "uppercase", letterSpacing: "0.04em" }}>Agent tokens</span>
      <span className="oxplow-tabular" style={{ color: "var(--text-secondary)", fontWeight: "var(--weight-medium)" }}>
        {formatTokens(totals.total_tokens)}
      </span>
      <span>
        · {totals.turns} turn{totals.turns === 1 ? "" : "s"}
      </span>
    </div>
  );
}
