/// A panel's parts drawn small: the body, the collapsed summary and the
/// header's choice toggles. The rail's sections and the dock's panels both
/// draw them, so a panel reads the same wherever it sits. `testIdPrefix`
/// names the surface (`rail-panel`, `dock-panel`).

import type { CSSProperties } from "react";

import { iconFor } from "../../icons.js";
import { PageTree } from "../../pages/PageTree.js";
import type { TabRef } from "../../tabs/tabState.js";
import type { OpenPanelRun, PanelChoice } from "./usePanelRuns.js";

/** The panel's body page, compact with its buttons. Its "+N more" calls
 *  `onMore`. `filled` shows every row, scrolling. */
export function PanelBody({
  body,
  streamId,
  onOpenPage,
  onMore,
  filled = false,
  testIdPrefix = "rail-panel",
}: {
  body: OpenPanelRun;
  streamId: string | null;
  onOpenPage(ref: TabRef): void;
  onMore(): void;
  filled?: boolean;
  testIdPrefix?: string;
}) {
  const style: CSSProperties = {
    padding: "4px 10px 8px",
    fontSize: "var(--text-xs)",
    ...(filled ? { flex: 1, minHeight: 0, overflowY: "auto" as const } : {}),
  };
  return (
    <div data-testid={`${testIdPrefix}-body`} className="ox-compact" style={style}>
      <PageTree run={body.run} runId={body.runId} compact partButtons uncapped={filled} onMore={onMore} streamId={streamId} onOpenPage={onOpenPage} />
    </div>
  );
}

/** The panel's collapsed page: the one-line summary, with no buttons. */
export function PanelCollapsed({
  collapsed,
  streamId,
  onOpenPage,
  testIdPrefix = "rail-panel",
}: {
  collapsed: OpenPanelRun;
  streamId: string | null;
  onOpenPage(ref: TabRef): void;
  testIdPrefix?: string;
}) {
  return (
    <div data-testid={`${testIdPrefix}-collapsed`} className="ox-compact" style={{ padding: "4px 10px 8px", fontSize: "var(--text-xs)" }}>
      <PageTree run={collapsed.run} runId={collapsed.runId} compact partButtons={false} streamId={streamId} onOpenPage={onOpenPage} />
    </div>
  );
}

/** A panel's choice param in its header: one small button per
 *  option, the one in force pressed. */
export function PanelChoiceToggle({ choice, onPick, testIdPrefix = "rail-panel" }: { choice: PanelChoice; onPick(value: string): void; testIdPrefix?: string }) {
  return (
    <span role="group" aria-label={choice.label ?? choice.name} style={{ display: "inline-flex", gap: 2 }}>
      {choice.options.map((o) => {
        const active = o.value === choice.value;
        // An option that names an icon is the icon alone, its label the
        // hover and the button's name.
        const Icon = iconFor(o.icon);
        return (
          <button
            key={o.value}
            type="button"
            data-testid={`${testIdPrefix}-choice-${choice.name}-${o.value}`}
            aria-pressed={active}
            title={Icon ? o.label : undefined}
            aria-label={Icon ? o.label : undefined}
            onClick={(e) => {
              e.stopPropagation();
              if (!active) onPick(o.value);
            }}
            style={{
              // The one in force is boxed in the accent over a faint
              // accent wash, so it reads as picked against the header.
              background: active ? "var(--rail-panel-choice-bg)" : "transparent",
              border: `1px solid ${active ? "var(--accent)" : "transparent"}`,
              borderRadius: 4,
              color: active ? "var(--text-primary)" : "var(--text-secondary)",
              cursor: active ? "default" : "pointer",
              fontSize: "var(--text-xs)",
              padding: Icon ? "2px 3px" : "0 4px",
              display: "inline-flex",
              alignItems: "center",
            }}
          >
            {Icon ? <Icon size={13} aria-hidden /> : o.label}
          </button>
        );
      })}
    </span>
  );
}

