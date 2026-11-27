/// The dock's icon row, at the right of the bottom bar: one button per
/// panel, the open one highlighted, a count pill on the icon's top-right
/// corner when the panel has a badge.

import { Fragment } from "react";

import type { TabRef } from "../../tabs/tabState.js";
import { ICONS, iconFor } from "../../icons.js";
import { dockStore, useDockState } from "./dockStore.js";
import { useDockPanels, type DockPanelDef } from "./dockPanels.js";

export function DockBar({ streamId = null, onOpenPage = () => {} }: { streamId?: string | null; onOpenPage?(ref: TabRef): void } = {}) {
  const panels = useDockPanels();
  const { open } = useDockState();
  return (
    <div data-testid="dock-bar" role="toolbar" aria-label="Dock" style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
      {panels.map((p) => (
        <Fragment key={p.id}>
          {p.Collapsed && open !== p.id ? <p.Collapsed streamId={streamId} onOpenPage={onOpenPage} /> : null}
          <DockButton panel={p} active={open === p.id} />
        </Fragment>
      ))}
    </div>
  );
}

function DockButton({ panel, active }: { panel: DockPanelDef; active: boolean }) {
  const badge = panel.useBadge();
  const Icon = iconFor(panel.icon) ?? ICONS.box;
  const title = badge ? `${panel.title} (${badge.count})` : panel.title;
  return (
    <button
      type="button"
      data-testid={`dock-${panel.id}`}
      title={title}
      aria-label={title}
      aria-pressed={active}
      onClick={() => dockStore.toggle(panel.id)}
      style={{
        position: "relative",
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        height: 26,
        minWidth: 26,
        padding: "0 6px",
        background: active ? "var(--accent-soft-bg, var(--surface-app))" : "transparent",
        border: active ? "1px solid var(--border-strong)" : "1px solid transparent",
        borderRadius: 4,
        color: active ? "var(--text-primary)" : "var(--text-secondary)",
        cursor: "pointer",
      }}
    >
      <Icon size={16} aria-hidden />
      {badge && badge.count > 0 ? (
        <span
          data-testid={`dock-${panel.id}-badge`}
          data-tone={badge.tone ?? "accent"}
          // On the icon's top-right corner, overlapping it, ringed in the
          // bar's color so it reads as the icon's count.
          style={{
            position: "absolute",
            top: -4,
            right: -5,
            minWidth: 14,
            padding: "0 4px",
            borderRadius: 7,
            fontSize: "var(--text-xs)",
            lineHeight: "14px",
            fontWeight: 600,
            textAlign: "center",
            boxShadow: "0 0 0 2px var(--bg-2)",
            pointerEvents: "none",
            color: "var(--bg-1)",
            background: badge.tone === "danger" ? "var(--severity-critical)" : "var(--accent)",
          }}
        >
          {badge.count}
        </span>
      ) : null}
    </button>
  );
}
