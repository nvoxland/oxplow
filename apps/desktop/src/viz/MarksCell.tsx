import { Fragment, type CSSProperties } from "react";

import { toneColor } from "./vizIcons.js";
import type { Mark } from "./vizModel.js";

/** Short marks drawn tight, each in its tone, its hint on hover, a mark
 *  with `gap` starting a new group after a dot: a `marks` column's value, a
 *  path tree's status letters (A M D …). */
export function MarksCell({ marks, style }: { marks: readonly Mark[]; style?: CSSProperties }) {
  return (
    <span data-testid="viz-marks" style={{ display: "inline-flex", gap: 4, flexShrink: 0, fontFamily: "var(--font-mono, monospace)", fontSize: "0.92em", ...style }}>
      {marks.map((m, i) => (
        <Fragment key={i}>
          {m.gap && i > 0 ? (
            <span data-testid="viz-mark-gap" aria-hidden style={{ color: "var(--text-muted)", padding: "0 2px" }}>
              ·
            </span>
          ) : null}
          <span data-testid="viz-mark" data-tone={m.tone ?? undefined} title={m.hint} style={{ color: toneColor(m.tone) ?? undefined, fontWeight: 600 }}>
            {m.text}
          </span>
        </Fragment>
      ))}
    </span>
  );
}
