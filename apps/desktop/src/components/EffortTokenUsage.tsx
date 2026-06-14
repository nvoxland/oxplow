import { useEffect, useState } from "react";

import {
  type AgentTokenUsage,
  listTokenUsageForEffort,
  subscribeOxplowEvents,
} from "../api.js";
import { formatTokens } from "../tokens.js";

/**
 * Per-effort agent token usage (tsk104): one row per turn parsed from the
 * agent transcript on Stop, with summed totals. Tokens-only for now — the
 * actual per-turn model is captured so cost can be layered on later.
 *
 * Self-hides when the effort has no usage rows (no Claude Stop captured yet,
 * or a non-Claude agent). Live-updates on `agentTokenUsageChanged` for this
 * effort. Mirrors the AgentNudgesBlock disclosure pattern.
 */
export function EffortTokenUsageBlock({ effortId }: { effortId: string }) {
  const [rows, setRows] = useState<AgentTokenUsage[]>([]);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      void listTokenUsageForEffort(effortId).then((r) => {
        if (!cancelled) setRows(r);
      });
    };
    load();
    const unsub = subscribeOxplowEvents((event) => {
      if (event.kind !== "agentTokenUsageChanged") return;
      if (event.effortId !== effortId) return;
      load();
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [effortId]);

  if (rows.length === 0) return null;

  const sum = rows.reduce(
    (a, r) => ({
      input: a.input + r.input_tokens,
      output: a.output + r.output_tokens,
      cacheW: a.cacheW + r.cache_creation_input_tokens,
      cacheR: a.cacheR + r.cache_read_input_tokens,
    }),
    { input: 0, output: 0, cacheW: 0, cacheR: 0 },
  );
  const total = sum.input + sum.output + sum.cacheW + sum.cacheR;
  // Newest row carries the most recent model (rows are newest-first).
  const model = rows.find((r) => r.model)?.model ?? null;

  const mutedStyle: React.CSSProperties = {
    fontSize: "var(--text-xs)",
    color: "var(--text-muted)",
  };

  return (
    <div
      data-testid={`effort-token-usage-${effortId}`}
      style={{ display: "flex", flexDirection: "column", gap: 6 }}
    >
      <div style={{ display: "flex", alignItems: "baseline", gap: 8, flexWrap: "wrap" }}>
        <span
          style={{
            fontSize: "var(--text-xs)",
            color: "var(--text-muted)",
            textTransform: "uppercase",
            letterSpacing: "0.04em",
          }}
        >
          Tokens
        </span>
        <span
          data-testid={`effort-token-total-${effortId}`}
          style={{ fontSize: "var(--text-sm)", fontWeight: "var(--weight-medium)", color: "var(--text-secondary)" }}
        >
          {formatTokens(total)} total
        </span>
        <span style={mutedStyle}>
          {rows.length} turn{rows.length === 1 ? "" : "s"}
        </span>
        {model ? <span style={{ ...mutedStyle, fontFamily: "var(--font-mono)" }}>{model}</span> : null}
      </div>
      <div className="oxplow-tabular" style={{ display: "flex", gap: 12, ...mutedStyle }}>
        <span title="Fresh input tokens">in {formatTokens(sum.input)}</span>
        <span title="Output tokens">out {formatTokens(sum.output)}</span>
        <span title="Cache-write (creation) tokens">cache-w {formatTokens(sum.cacheW)}</span>
        <span title="Cache-read tokens">cache-r {formatTokens(sum.cacheR)}</span>
      </div>
    </div>
  );
}
