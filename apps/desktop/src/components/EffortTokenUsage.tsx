import { useEffect, useState } from "react";

import {
  type AgentTokenUsage,
  listTokenUsageForEffort,
  subscribeOxplowEvents,
} from "../api.js";
import { formatTokens } from "../tokens.js";

/** Prompts longer than this are truncated until the row is expanded. */
const PROMPT_PREVIEW_LEN = 140;

/**
 * Per-effort agent token usage (tsk104 + tsk143): one row per agent TURN
 * parsed from the transcript on Stop. Each turn carries the human-authored
 * prompt that opened it (pure observation — read from the transcript, never
 * generated), the model, and the turn's token usage. The panel is a per-turn
 * LOG so an effort review can see what was ASKED next to what it cost.
 *
 * Self-hides when the effort has no usage rows (no Claude Stop captured yet,
 * or a non-Claude agent). Live-updates on `agentTokenUsageChanged` for this
 * effort. Long prompts are collapsible so the log stays scannable.
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
      <ol
        data-testid={`effort-turn-log-${effortId}`}
        style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: 4 }}
      >
        {rows.map((row) => (
          <TurnRow key={row.id} row={row} />
        ))}
      </ol>
    </div>
  );
}

/** One turn in the log: collapsible prompt + model + token total. */
export function TurnRow({ row }: { row: AgentTokenUsage }) {
  const [expanded, setExpanded] = useState(false);
  const prompt = row.prompt?.trim() ?? "";
  const isLong = prompt.length > PROMPT_PREVIEW_LEN;
  const shown = !isLong || expanded ? prompt : `${prompt.slice(0, PROMPT_PREVIEW_LEN)}…`;
  const turnTotal =
    row.input_tokens + row.output_tokens + row.cache_creation_input_tokens + row.cache_read_input_tokens;

  const metaStyle: React.CSSProperties = {
    fontSize: "var(--text-xs)",
    color: "var(--text-muted)",
    display: "flex",
    gap: 8,
    flexShrink: 0,
  };

  return (
    <li
      data-testid={`effort-turn-${row.id}`}
      style={{
        display: "flex",
        alignItems: "baseline",
        justifyContent: "space-between",
        gap: 8,
        padding: "4px 0",
        borderTop: "1px solid var(--border-subtle)",
      }}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 2, minWidth: 0, flex: 1 }}>
        {prompt ? (
          <button
            type="button"
            data-testid={`effort-turn-prompt-${row.id}`}
            onClick={isLong ? () => setExpanded((v) => !v) : undefined}
            title={isLong ? (expanded ? "Collapse prompt" : "Expand prompt") : undefined}
            style={{
              all: "unset",
              cursor: isLong ? "pointer" : "default",
              fontSize: "var(--text-sm)",
              color: "var(--text-secondary)",
              whiteSpace: expanded ? "pre-wrap" : "normal",
              wordBreak: "break-word",
            }}
          >
            {shown}
          </button>
        ) : (
          <span style={{ fontSize: "var(--text-sm)", color: "var(--text-muted)", fontStyle: "italic" }}>
            (no prompt)
          </span>
        )}
      </div>
      <span className="oxplow-tabular" style={metaStyle}>
        {row.model ? <span style={{ fontFamily: "var(--font-mono)" }}>{row.model}</span> : null}
        <span title="Total tokens this turn">{formatTokens(turnTotal)}</span>
      </span>
    </li>
  );
}
