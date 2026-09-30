/// What to ask about the ref a page shows (P6.D2): the catalog's prompts
/// `about` its kind, each an Ask that puts `[oxplow ref …] <question>` in
/// the agent's input — never sent. Renders nothing when there are none.
import type { CSSProperties } from "react";

import { insertIntoAgent } from "../../agent-input-bus.js";
import { kindOf } from "../../refs/ref.js";
import { askText, promptsAbout } from "./promptModel.js";
import { usePromptCatalog } from "./usePromptCatalog.js";

export function SuggestedPrompts({
  refId,
  streamId,
  vertical = false,
  onAsked,
}: {
  refId: string;
  streamId: string | null;
  /** A list (the nav bar's Ask menu) rather than a row of chips. */
  vertical?: boolean;
  onAsked?(): void;
}) {
  const catalog = usePromptCatalog(streamId);
  const kind = kindOf(refId);
  const prompts = kind ? promptsAbout(catalog, kind) : [];
  if (prompts.length === 0) return null;
  return (
    <div data-testid="suggested-prompts" style={vertical ? columnStyle : rowStyle}>
      <span style={{ color: "var(--text-secondary)" }}>{vertical ? "Suggested:" : "Ask:"}</span>
      {prompts.map((p) => (
        <button
          key={`${p.source.name}:${p.prompt}`}
          type="button"
          style={chipStyle}
          title="Put this question in the agent's input (it isn't sent)"
          onClick={() => {
            insertIntoAgent(askText(p.prompt, refId));
            onAsked?.();
          }}
        >
          {p.prompt}
        </button>
      ))}
    </div>
  );
}

const rowStyle: CSSProperties = {
  display: "flex",
  flexWrap: "wrap",
  alignItems: "center",
  gap: 6,
  fontSize: "var(--text-xs)",
  margin: "4px 0 8px",
};
const columnStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  alignItems: "stretch",
  gap: 4,
  fontSize: "var(--text-xs)",
  borderTop: "1px solid var(--border-subtle)",
  paddingTop: 4,
};
export const chipStyle: CSSProperties = {
  border: "1px solid var(--border-subtle)",
  borderRadius: 12,
  padding: "2px 8px",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  fontSize: "var(--text-xs)",
  cursor: "pointer",
};
