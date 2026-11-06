/// A command asked for the person's confirmation (P6.B2): what it is and
/// does — its summary from `get_command`, destructive ones marked — then
/// Run (focused; Enter) or Cancel (Escape). Shared by page actions and form
/// pages; the caller runs the command again confirmed.

import type { CSSProperties } from "react";
import { useEffect, useState } from "react";

import { getCommand } from "../api.js";
import type { CommandSpec } from "../tauri-bridge/generated/bindings.js";
import { TabModal } from "./TabModal.js";

/** A confirmation that comes from within a tab: the tab's modal
 *  (`TabModal`), over that tab's body only. */
export function CommandConfirm(props: ConfirmProps) {
  return (
    <TabModal title={`Confirm ${props.label}`} onClose={props.onCancel} testId={`${props.testIdPrefix ?? "command-confirm"}-modal`} width="min(420px, 100%)">
      <ConfirmBox {...props} bare />
    </TabModal>
  );
}

interface ConfirmProps {
  /** What the person pressed (`Finish`). */
  label: string;
  /** The command it runs (`oxplow.work_item.transition`). */
  command: string;
  onConfirm(): void;
  onCancel(): void;
  /** What it asks about beyond itself, one line each (the permissions a
   *  deploy adds), from the bus's `NEEDS_CONFIRMATION`. */
  details?: string[];
  testIdPrefix?: string;
}

/** The confirmation itself, with no modal around it: where the command
 *  was run from outside any tab (`PersonCommandConfirm`, floating over
 *  the window). */
export function ConfirmBox({
  label,
  command,
  onConfirm,
  onCancel,
  details = [],
  testIdPrefix = "command-confirm",
  bare = false,
}: ConfirmProps & { bare?: boolean }) {
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
  const told = confirmTold(spec, details);
  const destructive = told.warn;
  return (
    <div
      data-testid={testIdPrefix}
      role="alertdialog"
      aria-label={`Confirm ${label}`}
      style={bare ? { ...boxStyle, border: "none", padding: 0, marginBottom: 0 } : destructive ? { ...boxStyle, borderColor: "var(--severity-critical)" } : boxStyle}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          // The confirmation's own cancel: a container's Escape mustn't also fire.
          e.stopPropagation();
          onCancel();
        }
      }}
    >
      <div style={{ fontSize: "var(--text-sm)" }}>
        <strong>{label}</strong>
        {told.warn ? <span style={{ color: "var(--severity-critical)" }}> — this removes or rewrites something</span> : null}
      </div>
      {told.sentence ? <div data-testid={`${testIdPrefix}-says`} style={{ fontSize: "var(--text-sm)", color: "var(--text-secondary)" }}>{told.sentence}</div> : null}
      {told.list.length > 0 ? (
        <ul data-testid={`${testIdPrefix}-details`} style={detailsStyle}>
          {told.list.map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
      ) : null}
      <div style={{ display: "flex", gap: 6, justifyContent: "flex-end" }}>
        <button type="button" className="ox-button" data-testid={`${testIdPrefix}-cancel`} onClick={onCancel}>
          Cancel
        </button>
        <button type="button" className="ox-button ox-primary" data-testid={`${testIdPrefix}-run`} autoFocus onClick={onConfirm}>
          {label}
        </button>
      </div>
    </div>
  );
}

/** What the box tells the person: a command's own sentence (which the bus
 *  sent as its one detail), or its summary, its input's lines, and the
 *  generic warning when it is destructive. */
export function confirmTold(
  spec: Pick<CommandSpec, "summary" | "confirm" | "confirm_says"> | null,
  details: readonly string[],
): { warn: boolean; sentence: string | null; list: readonly string[] } {
  if (spec?.confirm_says) return { warn: false, sentence: details[0] ?? spec.confirm_says, list: [] };
  return { warn: spec?.confirm === "destructive", sentence: spec?.summary ?? null, list: details };
}

const detailsStyle: CSSProperties = {
  margin: 0,
  paddingLeft: 18,
  fontSize: "var(--text-xs)",
};

const boxStyle: CSSProperties = {
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  padding: 8,
  display: "flex",
  flexDirection: "column",
  gap: 6,
  marginBottom: 6,
};
