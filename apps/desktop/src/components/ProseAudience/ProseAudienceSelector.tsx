import { useEffect, useRef, useState } from "react";
import { PROSE_AUDIENCES, type ProseAudience } from "../../tabs/proseAudience.js";

const LABELS: Record<ProseAudience, string> = {
  developer: "Developer",
  executive: "Executive",
  terse: "Terse",
};

export interface ProseAudienceSelectorProps {
  value: ProseAudience;
  onChange(audience: ProseAudience): void;
  /** Which variants the backend actually returned. A variant that's
   *  absent renders muted (and selecting it falls back to developer);
   *  developer is always available. Defaults to all-available. */
  available?: Record<ProseAudience, boolean>;
}

/**
 * Page-level dropdown that picks which audience variant of a prose body
 * is shown — Developer / Executive / Terse. A trigger button shows the
 * current audience; clicking opens a popover list. Dumb + props-driven;
 * the page wrapper wires it to the per-page `useProseAudience` store.
 */
export function ProseAudienceSelector({ value, onChange, available }: ProseAudienceSelectorProps) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const popoverRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Node;
      if (popoverRef.current?.contains(target)) return;
      if (triggerRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    const onScroll = () => setOpen(false);
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKey);
    window.addEventListener("scroll", onScroll, true);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", onScroll, true);
    };
  }, [open]);

  return (
    <div data-testid="prose-audience-selector" style={{ position: "relative", display: "inline-flex" }}>
      <button
        ref={triggerRef}
        type="button"
        data-testid="prose-audience-trigger"
        aria-haspopup="listbox"
        aria-expanded={open}
        title="Prose audience"
        onClick={() => setOpen((v) => !v)}
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: 6,
          border: "1px solid var(--border-subtle)",
          background: "var(--surface-card)",
          color: "var(--text-primary)",
          padding: "4px 10px",
          borderRadius: 4,
          fontSize: "var(--text-xs)",
          cursor: "pointer",
          whiteSpace: "nowrap",
        }}
      >
        {LABELS[value]}
        <span aria-hidden style={{ color: "var(--text-secondary)" }}>
          {open ? "▾" : "▸"}
        </span>
      </button>
      {open ? (
        <div
          ref={popoverRef}
          data-testid="prose-audience-popover"
          role="listbox"
          style={{
            position: "absolute",
            top: "calc(100% + 4px)",
            right: 0,
            minWidth: 160,
            background: "var(--surface-card)",
            border: "1px solid var(--border-subtle)",
            borderRadius: 6,
            boxShadow: "0 4px 12px rgba(0,0,0,0.18)",
            padding: 4,
            zIndex: 10,
            fontSize: "var(--text-xs)",
          }}
        >
          {PROSE_AUDIENCES.map((audience) => {
            const active = audience === value;
            const hasVariant = available ? available[audience] : true;
            return (
              <button
                key={audience}
                type="button"
                role="option"
                aria-selected={active}
                data-testid={`prose-audience-option-${audience}`}
                data-active={active ? "" : undefined}
                onClick={() => {
                  setOpen(false);
                  if (!active) onChange(audience);
                }}
                title={hasVariant ? LABELS[audience] : `No ${LABELS[audience].toLowerCase()} variant — showing developer`}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 8,
                  width: "100%",
                  textAlign: "left",
                  padding: "4px 8px",
                  background: active ? "var(--surface-tab-active, var(--surface-rail))" : "transparent",
                  border: "none",
                  borderRadius: 4,
                  color: active
                    ? "var(--accent-fg, var(--text-primary))"
                    : hasVariant
                      ? "var(--text-primary)"
                      : "var(--text-disabled, var(--text-secondary))",
                  opacity: hasVariant || active ? 1 : 0.55,
                  fontWeight: active ? 600 : 400,
                  cursor: "pointer",
                  whiteSpace: "nowrap",
                }}
              >
                <span style={{ display: "inline-block", width: 12 }}>{active ? "✓" : ""}</span>
                {LABELS[audience]}
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
