/// The dock's icon row, at the left of the bottom bar: one button per
/// panel, the open one highlighted, a count pill when the panel has a badge.

import { ICONS } from "../../icons.js";
import { dockStore, useDockState } from "./dockStore.js";
import { useDockPanels, type DockPanelDef } from "./dockPanels.js";

export function DockBar() {
  const panels = useDockPanels();
  const { open } = useDockState();
  return (
    <div data-testid="dock-bar" role="toolbar" aria-label="Dock" style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
      {panels.map((p) => (
        <DockButton key={p.id} panel={p} active={open === p.id} />
      ))}
    </div>
  );
}

function DockButton({ panel, active }: { panel: DockPanelDef; active: boolean }) {
  const badge = panel.useBadge();
  const Icon = ICONS[panel.icon] ?? ICONS["box"]!;
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
          style={{
            marginLeft: 4,
            minWidth: 14,
            padding: "0 4px",
            borderRadius: 7,
            fontSize: 10,
            lineHeight: "14px",
            fontWeight: 600,
            textAlign: "center",
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
