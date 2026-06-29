import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";

import { formatFullDateTime } from "../format.js";

/** One selectable snapshot in an endpoint dropdown. `gitCommit` (when set)
 *  is shown as a short sha beside the capture time. */
export interface EndpointSnapshotOption {
  snapshotId: number;
  /** Capture time (ISO) — rendered as a full date+time in the menu. */
  createdAt: string;
  gitCommit: string | null;
}

/**
 * A compact dropdown for one end of a diff range (Start / End). The closed
 * trigger shows a short, time-only label so it sits on one line beside its
 * "Start"/"End" caption; opening it lists up to ~20 recent snapshots with a
 * full date+time (and the pinned commit's short sha, when any). Picking one
 * fires `onPick(snapshotId)` — the caller rescopes the diff. The menu closes
 * on pick, Escape, or an outside click. Disabled when there's nothing to pick.
 */
export function EndpointPicker({
  testId,
  ariaLabel,
  triggerText,
  currentSnapshotId,
  options,
  onPick,
}: {
  testId: string;
  ariaLabel: string;
  /** Short (time-only) label for the closed trigger. */
  triggerText: string;
  /** Snapshot id of the currently-selected endpoint, highlighted in the
   *  menu (null when the endpoint isn't one of the listed snapshots). */
  currentSnapshotId: number | null;
  options: EndpointSnapshotOption[];
  onPick(snapshotId: number): void;
}) {
  const [open, setOpen] = useState(false);
  const btnRef = useRef<HTMLButtonElement | null>(null);
  const popRef = useRef<HTMLDivElement | null>(null);
  const [coords, setCoords] = useState<CSSProperties>({});
  const disabled = options.length === 0;

  // Position the popover under the trigger with `fixed` coords so it escapes
  // the details panel's clipping/sticky overflow (mirrors BranchPicker).
  useEffect(() => {
    if (!open) return;
    function place() {
      const btn = btnRef.current;
      if (!btn) return;
      const rect = btn.getBoundingClientRect();
      const width = Math.max(rect.width, 220);
      setCoords({
        position: "fixed",
        top: rect.bottom + 4,
        left: Math.max(8, Math.min(window.innerWidth - width - 8, rect.left)),
        width,
      });
    }
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    function onDocDown(e: MouseEvent) {
      const t = e.target as Node;
      if (popRef.current?.contains(t)) return;
      if (btnRef.current?.contains(t)) return;
      setOpen(false);
    }
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onDocDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onDocDown);
    };
  }, [open]);

  return (
    <span style={{ position: "relative", flex: 1, minWidth: 0 }}>
      <button
        ref={btnRef}
        type="button"
        data-testid={`${testId}-trigger`}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        style={{ ...triggerStyle, cursor: disabled ? "default" : "pointer", opacity: disabled ? 0.7 : 1 }}
      >
        <span style={{ flex: 1, textAlign: "left", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {triggerText}
        </span>
        {!disabled ? <span style={{ color: "var(--text-muted)", fontSize: 9 }}>▾</span> : null}
      </button>
      {open ? (
        <div ref={popRef} role="listbox" style={{ ...popoverStyle, ...coords }}>
          {options.map((o) => {
            const current = o.snapshotId === currentSnapshotId;
            return (
              <button
                key={o.snapshotId}
                type="button"
                role="option"
                aria-selected={current}
                data-testid={`${testId}-option-${o.snapshotId}`}
                onClick={() => {
                  onPick(o.snapshotId);
                  setOpen(false);
                }}
                style={{ ...optionStyle, background: current ? "var(--surface-hover, rgba(255,255,255,0.06))" : "transparent" }}
              >
                <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {formatFullDateTime(o.createdAt)}
                </span>
                {o.gitCommit ? (
                  <span style={{ fontFamily: "var(--mono, monospace)", fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>
                    {o.gitCommit.slice(0, 7)}
                  </span>
                ) : null}
                {current ? <span style={{ fontSize: 10, color: "var(--accent, #4aa3ff)" }}>●</span> : null}
              </button>
            );
          })}
        </div>
      ) : null}
    </span>
  );
}

const triggerStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  width: "100%",
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  padding: "3px 8px",
  color: "var(--text-secondary)",
  fontFamily: "inherit",
  fontSize: "var(--text-sm)",
  boxSizing: "border-box",
};

const popoverStyle: CSSProperties = {
  maxHeight: 320,
  overflowY: "auto",
  background: "var(--surface-card, var(--bg))",
  border: "1px solid var(--border-strong, var(--border-subtle))",
  borderRadius: 6,
  boxShadow: "0 8px 24px rgba(0,0,0,0.45)",
  padding: 4,
  display: "flex",
  flexDirection: "column",
  gap: 1,
  zIndex: 1200,
};

const optionStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  width: "100%",
  background: "transparent",
  border: "none",
  borderRadius: 3,
  padding: "4px 8px",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: "var(--text-sm)",
  color: "var(--text-primary)",
  textAlign: "left",
};
