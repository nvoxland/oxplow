import { ICONS } from "../icons.js";
import type { PageAction } from "./actions.js";

/** A row of actions, drawn the one way core draws them. One with an icon
 *  shows the icon alone, its label the hover and its accessible name. */
export function ActionButtons({ actions }: { actions: readonly PageAction[] }) {
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
            onClick={() => void a.run()}
          >
            {Icon ? <Icon size={14} aria-hidden /> : label}
          </button>
        );
      })}
    </div>
  );
}
