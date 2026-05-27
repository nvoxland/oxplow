import { PROSE_AUDIENCES, type ProseAudience } from "../../tabs/proseAudience.js";

const LABELS: Record<ProseAudience, string> = {
  developer: "Developer",
  executive: "Executive",
  caveman: "Caveman",
};

const GLYPHS: Record<ProseAudience, string> = {
  developer: "D",
  executive: "E",
  caveman: "C",
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
 * Page-level segmented control that picks which audience variant of a
 * prose body is shown — Developer / Executive / Caveman. Discrete
 * values, so it's a `radiogroup` of buttons (←/→ to move, Enter/Space
 * to commit), not a continuous slider. Dumb + props-driven; the page
 * wrapper wires it to the per-page `useProseAudience` store.
 */
export function ProseAudienceSelector({ value, onChange, available }: ProseAudienceSelectorProps) {
  const move = (dir: 1 | -1) => {
    const i = PROSE_AUDIENCES.indexOf(value);
    const next = PROSE_AUDIENCES[(i + dir + PROSE_AUDIENCES.length) % PROSE_AUDIENCES.length];
    onChange(next);
  };

  return (
    <div
      data-testid="prose-audience-selector"
      role="radiogroup"
      aria-label="Prose audience"
      onKeyDown={(e) => {
        if (e.key === "ArrowRight" || e.key === "ArrowDown") {
          e.preventDefault();
          move(1);
        } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
          e.preventDefault();
          move(-1);
        }
      }}
      style={{
        display: "inline-flex",
        alignItems: "stretch",
        border: "1px solid var(--border-subtle)",
        borderRadius: 4,
        overflow: "hidden",
        flexShrink: 0,
      }}
    >
      {PROSE_AUDIENCES.map((audience, i) => {
        const active = audience === value;
        const hasVariant = available ? available[audience] : true;
        return (
          <button
            key={audience}
            type="button"
            role="radio"
            aria-checked={active}
            data-testid={`prose-audience-option-${audience}`}
            data-active={active ? "" : undefined}
            tabIndex={active ? 0 : -1}
            onClick={() => onChange(audience)}
            title={
              hasVariant
                ? LABELS[audience]
                : `No ${LABELS[audience].toLowerCase()} variant — showing developer`
            }
            style={{
              border: "none",
              borderLeft: i === 0 ? "none" : "1px solid var(--border-subtle)",
              background: active ? "var(--surface-tab-active, var(--surface-rail))" : "transparent",
              color: active
                ? "var(--accent-fg, var(--text-primary))"
                : hasVariant
                  ? "var(--text-secondary)"
                  : "var(--text-disabled, var(--text-secondary))",
              opacity: hasVariant || active ? 1 : 0.45,
              fontWeight: active ? 600 : 400,
              fontSize: "var(--text-xs)",
              padding: "3px 10px",
              cursor: "pointer",
              whiteSpace: "nowrap",
            }}
          >
            <span aria-hidden style={{ marginRight: 4, fontVariantNumeric: "tabular-nums" }}>
              {GLYPHS[audience]}
            </span>
            {LABELS[audience]}
          </button>
        );
      })}
    </div>
  );
}
