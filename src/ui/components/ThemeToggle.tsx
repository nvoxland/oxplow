import { useEffect, useState } from "react";
import {
  getStoredThemePref,
  setStoredThemePref,
  subscribeThemePref,
  type ThemePreference,
} from "../theme.js";

const OPTIONS: Array<{ value: ThemePreference; label: string; title: string }> = [
  { value: "light", label: "☀", title: "Light" },
  { value: "dark", label: "☾", title: "Dark" },
  { value: "system", label: "⌘", title: "System" },
];

export interface ThemeToggleProps {
  /** Visual style — `compact` is a single icon-button cycling through options;
   *  `segmented` shows all three side-by-side. */
  variant?: "compact" | "segmented";
  className?: string;
  style?: React.CSSProperties;
}

export function ThemeToggle({ variant = "segmented", className, style }: ThemeToggleProps) {
  const [pref, setPref] = useState<ThemePreference>(() => getStoredThemePref() ?? "light");

  useEffect(() => {
    return subscribeThemePref((p) => setPref(p));
  }, []);

  if (variant === "compact") {
    const order: ThemePreference[] = ["light", "dark", "system"];
    const next = order[(order.indexOf(pref) + 1) % order.length];
    const current = OPTIONS.find((o) => o.value === pref);
    return (
      <button
        type="button"
        className={className}
        title={`Theme: ${current?.title ?? pref} (click for ${OPTIONS.find((o) => o.value === next)?.title})`}
        data-testid="theme-toggle-compact"
        onClick={() => setStoredThemePref(next)}
        style={{
          background: "var(--surface-elevated)",
          color: "var(--text-secondary)",
          border: "1px solid var(--border-subtle)",
          borderRadius: 6,
          width: 28,
          height: 28,
          display: "inline-flex",
          alignItems: "center",
          justifyContent: "center",
          cursor: "pointer",
          fontSize: 14,
          ...style,
        }}
      >
        {current?.label ?? "⌘"}
      </button>
    );
  }

  return (
    <div
      role="radiogroup"
      aria-label="Theme"
      className={className}
      data-testid="theme-toggle-segmented"
      style={{
        display: "inline-flex",
        gap: 0,
        background: "var(--surface-elevated)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 8,
        padding: 2,
        ...style,
      }}
    >
      {OPTIONS.map((opt) => {
        const active = pref === opt.value;
        return (
          <button
            key={opt.value}
            type="button"
            role="radio"
            aria-checked={active}
            data-testid={`theme-toggle-${opt.value}`}
            title={opt.title}
            onClick={() => setStoredThemePref(opt.value)}
            style={{
              background: active ? "var(--accent-soft-bg)" : "transparent",
              color: active ? "var(--accent)" : "var(--text-secondary)",
              border: "none",
              borderRadius: 6,
              padding: "4px 10px",
              cursor: "pointer",
              fontSize: 12,
              fontWeight: active ? 600 : 400,
              minWidth: 32,
            }}
          >
            <span aria-hidden style={{ marginRight: 4 }}>
              {opt.label}
            </span>
            {opt.title}
          </button>
        );
      })}
    </div>
  );
}
