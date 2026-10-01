/// The page nav bar's Commands (P6b.C4): extensions' `ui.commands` about
/// the page's ref (`placement: menu`), grouped under the provider or
/// extension. Each runs as the person through `personCommands` (which
/// asks first when the command asks). Hidden when nothing applies.
import type { CSSProperties } from "react";
import { useState } from "react";

import { personCommands } from "../personCommands.js";
import type { AskTarget } from "./Prompts/AskMenu.js";
import { bindRefInput, groupUiCommands, uiCommandsAbout } from "./uiCommands.js";
import { usePopoverDismiss } from "./usePopoverDismiss.js";
import { useUiCommands } from "./useUiCommands.js";

export function RefCommandsMenu({ target, buttonStyle }: { target: AskTarget; buttonStyle: CSSProperties }) {
  const all = useUiCommands(target.streamId);
  const [open, setOpen] = useState(false);
  const boxRef = usePopoverDismiss<HTMLDivElement>(open, () => setOpen(false));
  const commands = uiCommandsAbout(all, target.ref, "menu");
  if (commands.length === 0) return null;
  return (
    <div ref={boxRef} style={{ position: "relative", display: "inline-flex" }}>
      <button
        type="button"
        data-testid="page-nav-commands"
        title="Commands extensions offer for this page"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        style={buttonStyle}
      >
        Commands
      </button>
      {open ? (
        <div data-testid="page-nav-commands-menu" style={popoverStyle}>
          {groupUiCommands(commands).map((g) => (
            <div key={g.group} style={{ display: "flex", flexDirection: "column" }}>
              <div style={groupStyle}>{g.group}</div>
              {g.commands.map((c) => (
                <button
                  key={c.id}
                  type="button"
                  data-testid={`page-nav-command-${c.id}`}
                  style={itemStyle}
                  onClick={() => {
                    setOpen(false);
                    void personCommands.run(c.label, c.command, bindRefInput(c.input, target.ref));
                  }}
                >
                  {c.label}
                </button>
              ))}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

const popoverStyle: CSSProperties = {
  position: "absolute",
  top: "calc(100% + 4px)",
  right: 0,
  minWidth: 200,
  maxWidth: 320,
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  boxShadow: "0 4px 12px rgba(0,0,0,0.18)",
  padding: 6,
  zIndex: 10,
  fontSize: "var(--text-xs)",
  display: "flex",
  flexDirection: "column",
  gap: 6,
};
const groupStyle: CSSProperties = {
  color: "var(--text-secondary)",
  fontSize: "var(--text-xs)",
  textTransform: "uppercase",
  letterSpacing: 0.4,
  padding: "2px 6px",
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
