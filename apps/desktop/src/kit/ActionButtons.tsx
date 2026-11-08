import type { CSSProperties } from "react";
import { useId } from "react";

import { useActionAnchor, useRunner } from "../commands/RunnerContext.js";
import { ICONS } from "../icons.js";
import type { PageAction } from "./actions.js";
import { InlineError } from "./InlineError.js";

/** A row of actions, drawn the one way core draws them. One with an icon
 *  shows the icon alone, its label the hover and its accessible name.
 *  `buttonStyle` matches them to the controls beside them (the nav bar's);
 *  a primary one keeps the accent over it. Each runs through the person's
 *  runner (`commands/runForPerson.ts`): a failure shows beside its button. */
export function ActionButtons({ actions, buttonStyle }: { actions: readonly PageAction[]; buttonStyle?: CSSProperties }) {
  if (actions.length === 0) return null;
  return (
    <div className="ox-actions">
      {actions.map((a) => (
        <ActionButton key={a.id} action={a} buttonStyle={buttonStyle} />
      ))}
    </div>
  );
}

function ActionButton({ action: a, buttonStyle }: { action: PageAction; buttonStyle?: CSSProperties }) {
  const local = useId();
  const anchor = a.anchor ?? `action/${local}/${a.id}`;
  const failure = useActionAnchor(anchor);
  const { run } = useRunner();
  const Icon = a.icon ? ICONS[a.icon] : undefined;
  const label = a.busy ? `${a.label}…` : a.label;
  return (
    <>
      <button
        type="button"
        data-testid={`action-${a.id}`}
        className={a.primary ? "ox-button ox-primary" : "ox-button"}
        disabled={a.disabled || a.busy}
        title={Icon ? (a.title ? `${label}: ${a.title}` : label) : a.title}
        aria-label={Icon ? label : undefined}
        style={buttonStyle && a.primary ? { ...buttonStyle, ...accent } : buttonStyle}
        onClick={() => void run(a.label, async () => a.run(), { anchor })}
      >
        {Icon ? <Icon size={16} aria-hidden /> : label}
      </button>
      {failure ? <InlineError failure={failure} testId={`action-error-${a.id}`} /> : null}
    </>
  );
}

const accent: CSSProperties = { background: "var(--accent)", color: "#fff", borderColor: "transparent" };
