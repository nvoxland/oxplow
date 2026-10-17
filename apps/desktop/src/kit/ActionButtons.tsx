import type { PageAction } from "./actions.js";

/** A row of actions, drawn the one way core draws them. */
export function ActionButtons({ actions }: { actions: readonly PageAction[] }) {
  if (actions.length === 0) return null;
  return (
    <div className="ox-actions">
      {actions.map((a) => (
        <button
          key={a.id}
          type="button"
          data-testid={`action-${a.id}`}
          className={a.primary ? "ox-button ox-primary" : "ox-button"}
          disabled={a.disabled || a.busy}
          title={a.title}
          onClick={() => void a.run()}
        >
          {a.busy ? `${a.label}…` : a.label}
        </button>
      ))}
    </div>
  );
}
