/// An empty page or section (P6.D2, `.context/usability.md` → "Empty
/// states"): what would be here, one sentence on how it gets here, and
/// 1–3 prompts the person can hand the agent — each Ask fills the agent's
/// input, never sends. `compact` is the one-line form for rail sections.
import type { CSSProperties, ReactNode } from "react";

import { insertIntoAgent } from "../../agent-input-bus.js";
import { chipStyle } from "./SuggestedPrompts.js";

export function EmptyState({
  title,
  text,
  prompts = [],
  children,
  compact = false,
  testId,
}: {
  title: string;
  text?: ReactNode;
  prompts?: string[];
  /** Anything else the empty state offers (a "+ New" button). */
  children?: ReactNode;
  compact?: boolean;
  testId?: string;
}) {
  return (
    <div data-testid={testId} data-empty-state="" style={compact ? compactStyle : boxStyle}>
      <div style={compact ? undefined : { fontWeight: 600 }}>{title}</div>
      {text ? <div style={{ color: "var(--text-secondary)" }}>{text}</div> : null}
      {children}
      {prompts.length > 0 ? (
        <div style={{ display: "flex", flexWrap: "wrap", gap: 6, alignItems: "center" }}>
          {compact ? null : <span style={{ color: "var(--text-secondary)" }}>Ask the agent:</span>}
          {prompts.slice(0, 3).map((p) => (
            <button
              key={p}
              type="button"
              style={chipStyle}
              title="Put this in the agent's input (it isn't sent)"
              onClick={() => insertIntoAgent(p)}
            >
              {p}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

const boxStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 6,
  padding: "12px 0",
  color: "var(--text-primary)",
  fontSize: "var(--text-sm)",
};
const compactStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 4,
  color: "var(--text-muted)",
  fontSize: "var(--text-xs)",
};
