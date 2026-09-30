/// A command asked for the person's confirmation (P6.B2): what it is and
/// does — its summary from `get_command`, destructive ones marked — then
/// Run (focused; Enter) or Cancel (Escape). Shared by lens actions and form
/// lenses; the caller runs the command again confirmed.

import type { CSSProperties } from "react";
import { useEffect, useState } from "react";

import { getCommand } from "../api.js";
import type { CommandSpec } from "../tauri-bridge/generated/bindings.js";

export function CommandConfirm({
  label,
  command,
  onConfirm,
  onCancel,
  testIdPrefix = "command-confirm",
}: {
  /** What the person pressed (`Finish`). */
  label: string;
  /** The command it runs (`work_item.transition`). */
  command: string;
  onConfirm(): void;
  onCancel(): void;
  testIdPrefix?: string;
}) {
  const [spec, setSpec] = useState<CommandSpec | null>(null);
  useEffect(() => {
    let live = true;
    getCommand(command)
      .then((s) => {
        if (live) setSpec(s);
      })
      .catch(() => {
        // The summary is a courtesy; the name still says what runs.
      });
    return () => {
      live = false;
    };
  }, [command]);
  const destructive = spec?.confirm === "destructive";
  return (
    <div
      data-testid={testIdPrefix}
      role="alertdialog"
      aria-label={`Confirm ${label}`}
      style={destructive ? { ...boxStyle, borderColor: "var(--severity-critical)" } : boxStyle}
      onKeyDown={(e) => {
        if (e.key === "Escape") onCancel();
      }}
    >
      <div style={{ fontSize: "var(--text-sm)" }}>
        <strong>{label}</strong> runs <code>{command}</code>
        {destructive ? <span style={{ color: "var(--severity-critical)" }}> — it can't be undone</span> : null}.
      </div>
      {spec ? <div style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)" }}>{spec.summary}</div> : null}
      <div style={{ display: "flex", gap: 6 }}>
        <button type="button" data-testid={`${testIdPrefix}-run`} autoFocus onClick={onConfirm}>
          Run {label}
        </button>
        <button type="button" data-testid={`${testIdPrefix}-cancel`} onClick={onCancel}>
          Cancel
        </button>
      </div>
    </div>
  );
}

const boxStyle: CSSProperties = {
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  padding: 8,
  display: "flex",
  flexDirection: "column",
  gap: 6,
  marginBottom: 6,
};
