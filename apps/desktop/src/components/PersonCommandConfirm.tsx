/// Where a command the person ran from outside a page asks first
/// (`personCommands.ts`): the shared `CommandConfirm`, floating over the
/// app. Mounted once in `App`.
import type { CSSProperties } from "react";
import { useSyncExternalStore } from "react";

import { personCommands } from "../personCommands.js";
import { CommandConfirm } from "./CommandConfirm.js";

export function PersonCommandConfirm() {
  const pending = useSyncExternalStore(personCommands.subscribe, personCommands.pending);
  if (!pending) return null;
  return (
    <div style={hostStyle}>
      <CommandConfirm
        label={pending.label}
        command={pending.command}
        onConfirm={() => void personCommands.confirm()}
        onCancel={personCommands.cancel}
        testIdPrefix="person-command-confirm"
      />
    </div>
  );
}

const hostStyle: CSSProperties = {
  position: "fixed",
  top: 56,
  left: "50%",
  transform: "translateX(-50%)",
  zIndex: 1000,
  minWidth: 320,
  maxWidth: 520,
  background: "var(--surface-elevated)",
  borderRadius: 6,
  boxShadow: "0 8px 24px rgba(0, 0, 0, 0.35)",
};
