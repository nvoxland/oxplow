/// The page nav bar's Ask (P6.D1/D2): "Ask About This" puts the page's ref
/// in the agent's input; below it, the catalog's prompts about that kind
/// of ref, each asked about this one. Nothing is sent. Closes on Escape or
/// a click outside.
import type { CSSProperties } from "react";
import { useState } from "react";

import { formatContextMention } from "../../agent-context-ref.js";
import { insertIntoAgent } from "../../agent-input-bus.js";
import { usePopoverDismiss } from "../usePopoverDismiss.js";
import { SuggestedPrompts } from "./SuggestedPrompts.js";

export interface AskTarget {
  /** The page's canonical ref. */
  ref: string;
  /** Whose extensions' prompts to offer. */
  streamId: string | null;
}

export function AskMenu({ ask, buttonStyle }: { ask: AskTarget; buttonStyle: CSSProperties }) {
  const [open, setOpen] = useState(false);
  const boxRef = usePopoverDismiss<HTMLDivElement>(open, () => setOpen(false));
  return (
    <div ref={boxRef} style={{ position: "relative", display: "inline-flex" }}>
      <button
        type="button"
        data-testid="page-nav-ask"
        title="Ask the agent about this page (fills its input; nothing is sent)"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        style={buttonStyle}
      >
        Ask
      </button>
      {open ? (
        <div data-testid="page-nav-ask-menu" style={popoverStyle}>
          <button
            type="button"
            data-testid="page-nav-ask-this"
            autoFocus
            style={itemStyle}
            onClick={() => {
              insertIntoAgent(formatContextMention({ kind: "ref", ref: ask.ref }));
              setOpen(false);
            }}
          >
            Ask About This
          </button>
          <SuggestedPrompts refId={ask.ref} streamId={ask.streamId} vertical onAsked={() => setOpen(false)} />
        </div>
      ) : null}
    </div>
  );
}

const popoverStyle: CSSProperties = {
  position: "absolute",
  top: "calc(100% + 4px)",
  right: 0,
  minWidth: 240,
  maxWidth: 360,
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  boxShadow: "0 4px 12px rgba(0,0,0,0.18)",
  padding: 6,
  zIndex: 10,
  fontSize: "var(--text-xs)",
  display: "flex",
  flexDirection: "column",
  gap: 4,
};
const itemStyle: CSSProperties = {
  textAlign: "left",
  background: "none",
  border: "none",
  padding: "4px 6px",
  color: "var(--text-primary)",
  cursor: "pointer",
  fontSize: "var(--text-xs)",
};
