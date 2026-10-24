import type { CSSProperties } from "react";

import { ICONS } from "../icons.js";
import type { PageAction } from "./actions.js";

/** A row of actions, drawn the one way core draws them. One with an icon
 *  shows the icon alone, its label the hover and its accessible name.
 *  `buttonStyle` matches them to the controls beside them (the nav bar's);
 *  a primary one keeps the accent over it. */
export function ActionButtons({ actions, buttonStyle }: { actions: readonly PageAction[]; buttonStyle?: CSSProperties }) {
  if (actions.length === 0) return null;
  return (
    <div className="ox-actions">
      {actions.map((a) => {
        const Icon = a.icon ? ICONS[a.icon] : undefined;
        const label = a.busy ? `${a.label}…` : a.label;
        return (
          <button
            key={a.id}
            type="button"
            data-testid={`action-${a.id}`}
            className={a.primary ? "ox-button ox-primary" : "ox-button"}
            disabled={a.disabled || a.busy}
            title={Icon ? (a.title ? `${label}: ${a.title}` : label) : a.title}
            aria-label={Icon ? label : undefined}
            style={buttonStyle && a.primary ? { ...buttonStyle, ...accent } : buttonStyle}
            onClick={() => void a.run()}
          >
            {Icon ? <Icon size={16} aria-hidden /> : label}
          </button>
        );
      })}
    </div>
  );
}

const accent: CSSProperties = { background: "var(--accent)", color: "#fff", borderColor: "transparent" };
