import type { CSSProperties } from "react";
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { getOpErrorsStore } from "./opErrorsStore.js";
import { opErrorRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";

/**
 * Bottom-bar widget for async-operation failures (git push/pull, commit,
 * wiki save, file open, LSP, …). Renders nothing when there are no
 * errors; otherwise a red `⚠ N` chip that opens a popover listing every
 * error — click a row to open its detail page, `×` to dismiss, or "Clear
 * all". Shows ALL errors globally (not scoped to the current thread).
 *
 * Mirrors `BackgroundTaskIndicator`: same fixed-popover-above-the-button
 * placement and outside-click / Escape dismissal.
 */
export function OpErrorIndicator({
  onOpenPage,
  onDismiss,
  onClear,
}: {
  onOpenPage(ref: TabRef): void;
  onDismiss(id: string): void;
  onClear(): void;
}) {
  const store = getOpErrorsStore();
  const entries = useSyncExternalStore(store.subscribe, store.getSnapshot);
  const [open, setOpen] = useState(false);
  const btnRef = useRef<HTMLButtonElement | null>(null);
  const popRef = useRef<HTMLDivElement | null>(null);
  const [popoverCoords, setPopoverCoords] = useState<CSSProperties>({});

  useEffect(() => {
    if (!open) return;
    function place() {
      const btn = btnRef.current;
      if (!btn) return;
      const rect = btn.getBoundingClientRect();
      const width = 340;
      setPopoverCoords({
        position: "fixed",
        width,
        bottom: window.innerHeight - rect.top + 4,
        left: Math.max(8, Math.min(window.innerWidth - width - 8, rect.left)),
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
    function onDocClick(e: MouseEvent) {
      const t = e.target as Node;
      if (popRef.current?.contains(t)) return;
      if (btnRef.current?.contains(t)) return;
      setOpen(false);
    }
    window.addEventListener("keydown", onKey);
    window.addEventListener("mousedown", onDocClick);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mousedown", onDocClick);
    };
  }, [open]);

  // Close the popover automatically once the last error is gone.
  useEffect(() => {
    if (entries.length === 0 && open) setOpen(false);
  }, [entries.length, open]);

  if (entries.length === 0) return null;

  const unread = entries.filter((e) => !e.seen).length;

  return (
    <>
      <button
        type="button"
        ref={btnRef}
        data-testid="op-error-indicator"
        title={`${entries.length} error${entries.length === 1 ? "" : "s"} (click to expand)`}
        onClick={() => setOpen((v) => !v)}
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: 6,
          padding: "2px 8px",
          height: 22,
          background: unread > 0 ? "var(--diff-del-fg, #f85149)" : "transparent",
          border: "1px solid var(--diff-del-fg, #f85149)",
          borderRadius: 4,
          color: unread > 0 ? "var(--accent-on-accent, #fff)" : "var(--diff-del-fg, #f85149)",
          fontSize: 11,
          fontWeight: 600,
          cursor: "pointer",
        }}
      >
        <span aria-hidden>⚠</span>
        <span>{entries.length}</span>
      </button>
      {open && (
        <div
          ref={popRef}
          data-testid="op-error-popover"
          style={{
            ...popoverCoords,
            background: "var(--surface-elevated, #1e1e1e)",
            border: "1px solid var(--border)",
            borderRadius: 6,
            boxShadow: "0 6px 24px rgba(0,0,0,0.4)",
            padding: 8,
            zIndex: 1000,
            maxHeight: 400,
            overflow: "auto",
          }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              fontSize: 11,
              color: "var(--muted)",
              padding: "4px 6px 6px",
              borderBottom: "1px solid var(--border)",
              marginBottom: 4,
            }}
          >
            <span style={{ flex: 1 }}>Errors ({entries.length})</span>
            <button
              type="button"
              data-testid="op-error-clear"
              onClick={() => { onClear(); setOpen(false); }}
              title="Clear all errors"
              style={{ background: "transparent", border: "none", color: "var(--text-secondary)", cursor: "pointer", fontSize: 10, padding: "0 4px" }}
            >
              Clear all
            </button>
          </div>
          {entries.map((entry) => (
            <div key={entry.id} style={{ display: "flex", alignItems: "center", gap: 4 }}>
              <button
                type="button"
                data-testid={`op-error-row-${entry.id}`}
                title={entry.stderr || entry.message || entry.label}
                onClick={() => {
                  store.markSeen(entry.id);
                  onOpenPage(opErrorRef(entry.id));
                  setOpen(false);
                }}
                style={{
                  flex: 1,
                  display: "flex",
                  alignItems: "center",
                  gap: 8,
                  background: "transparent",
                  border: "none",
                  borderRadius: 4,
                  padding: "6px",
                  textAlign: "left",
                  cursor: "pointer",
                  color: entry.seen ? "var(--text-secondary)" : "var(--text-primary)",
                  fontSize: "var(--text-xs)",
                  minWidth: 0,
                }}
              >
                <span
                  aria-hidden
                  style={{
                    width: 6,
                    height: 6,
                    borderRadius: "50%",
                    flexShrink: 0,
                    background: entry.seen ? "transparent" : "var(--diff-del-fg, #f85149)",
                    border: entry.seen ? "1px solid var(--diff-del-fg, #f85149)" : "none",
                  }}
                />
                <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {entry.label}
                </span>
              </button>
              <button
                type="button"
                data-testid={`op-error-dismiss-${entry.id}`}
                title="Dismiss"
                onClick={(e) => { e.stopPropagation(); onDismiss(entry.id); }}
                style={{ background: "transparent", border: "none", color: "var(--text-secondary)", cursor: "pointer", padding: "2px 4px", fontSize: 11 }}
              >
                ×
              </button>
            </div>
          ))}
        </div>
      )}
    </>
  );
}
