import type { CSSProperties, ReactNode } from "react";
import { Pin, PinOff } from "lucide-react";

import type { ExtensionPanel } from "../tauri-bridge/generated/bindings.js";
import { OpenPageLink } from "../kit/OpenPageLink.js";
import { Slideover } from "../components/Slideover.js";
import { PanelBody, PanelChoiceToggle } from "../components/Panels/PanelParts.js";
import { extensionPanelId, panelOpenRef } from "../components/Panels/panelLayout.js";
import { usePanelLayout } from "../components/Panels/PanelLayoutContext.js";
import { usePanelRuns } from "../components/Panels/PanelRunsContext.js";
import type { PanelRuns } from "../components/Panels/usePanelRuns.js";
import { useExtensions } from "../extensionsStore.js";
import { ICONS, iconFor } from "../icons.js";
import { useAppPreference } from "../prefs/remembered.js";
import type { TabRef } from "../tabs/tabState.js";
import { useState } from "react";

const NO_RUNS: PanelRuns = { body: null, badge: null, collapsed: null, count: null, choices: [] };

/** A rail panel as a shell without a rail shows it. */
export interface TrayPanel {
  /** Its layout id (`ext:<extension>/<panel>`). */
  id: string;
  panel: ExtensionPanel;
  runs: PanelRuns;
  open: TabRef;
}

/** The rail's panels in the device's rail order (`usePanelLayout`: the
 *  person's order and the panels they show), with their runs. */
export function useTrayPanels(): { panels: TrayPanel[]; choose(panelId: string, param: string, value: string): void } {
  const { panels, runs, choose } = usePanelRuns();
  const { layout } = usePanelLayout();
  const extensions = useExtensions();
  const byId = new Map(panels.filter((p) => !p.dock).map((p) => [extensionPanelId(p), p] as const));
  return {
    panels: layout.order.flatMap((id) => {
      const panel = byId.get(id);
      return panel ? [{ id, panel, runs: runs[panel.id] ?? NO_RUNS, open: panelOpenRef(panel, extensions) }] : [];
    }),
    choose,
  };
}

/** The panel this device keeps beside the page, by layout id. */
export function usePinnedPanel(): [string | null, (id: string | null) => void] {
  return useAppPreference<string | null>("tablet.pinnedPanel", null);
}

/** The tablet's tray: a button per rail panel (its icon, its name, its
 *  count), then the dock's bar (`dock`: Alerts, Context, the extensions'
 *  dock panels). A tap opens the panel as a sheet. */
export function PanelTray({ dock, streamId, onOpenPage }: { dock: ReactNode; streamId: string | null; onOpenPage(ref: TabRef): void }) {
  const { panels, choose } = useTrayPanels();
  const [pinned, setPinned] = usePinnedPanel();
  return <PanelTrayView panels={panels} choose={choose} pinned={pinned} setPinned={setPinned} dock={dock} streamId={streamId} onOpenPage={onOpenPage} />;
}

/** The tray drawn from its panels; exported for tests. */
export function PanelTrayView({
  panels,
  choose,
  pinned,
  setPinned,
  dock,
  streamId,
  onOpenPage,
}: {
  panels: TrayPanel[];
  choose(panelId: string, param: string, value: string): void;
  pinned: string | null;
  setPinned(id: string | null): void;
  dock: ReactNode;
  streamId: string | null;
  onOpenPage(ref: TabRef): void;
}) {
  const [sheet, setSheet] = useState<string | null>(null);
  const open = panels.find((p) => p.id === sheet) ?? null;
  return (
    <div style={trayStyle}>
      <div role="toolbar" aria-label="Panels" data-testid="tablet-panels" style={buttons}>
        {panels.map((p) => {
          const Icon = iconFor(p.panel.icon) ?? ICONS.box;
          return (
            <button
              key={p.id}
              type="button"
              data-testid={`tablet-panel-${p.panel.id}`}
              aria-pressed={pinned === p.id}
              style={{ ...panelButton, ...(pinned === p.id ? pinnedButton : null) }}
              onClick={() => setSheet(p.id)}
            >
              <Icon size={16} aria-hidden />
              <span>{p.panel.title}</span>
              {p.runs.count ? (
                <span data-testid={`tablet-panel-count-${p.panel.id}`} className="ox-badge">
                  {p.runs.count}
                </span>
              ) : null}
            </button>
          );
        })}
      </div>
      <div style={{ marginLeft: "auto" }}>{dock}</div>
      {open ? (
        <Slideover
          open
          onClose={() => setSheet(null)}
          title={open.panel.title}
          testId="tablet-panel-sheet"
          actions={
            <>
              <PinButton pinned={pinned === open.id} onToggle={() => { setPinned(pinned === open.id ? null : open.id); setSheet(null); }} />
              <OpenPageLink to={open.open} name={open.panel.title} onNavigate={(ref) => { setSheet(null); onOpenPage(ref); }} />
            </>
          }
        >
          <PanelContent panel={open} choose={choose} streamId={streamId} onOpenPage={(ref) => { setSheet(null); onOpenPage(ref); }} />
        </Slideover>
      ) : null}
    </div>
  );
}

/** The pinned panel, a column beside the page; nothing when none is. */
export function PinnedPanel({ streamId, onOpenPage }: { streamId: string | null; onOpenPage(ref: TabRef): void }) {
  const { panels, choose } = useTrayPanels();
  const [pinned, setPinned] = usePinnedPanel();
  return <PinnedPanelView panel={panels.find((p) => p.id === pinned) ?? null} choose={choose} unpin={() => setPinned(null)} streamId={streamId} onOpenPage={onOpenPage} />;
}

export function PinnedPanelView({
  panel,
  choose,
  unpin,
  streamId,
  onOpenPage,
}: {
  panel: TrayPanel | null;
  choose(panelId: string, param: string, value: string): void;
  unpin(): void;
  streamId: string | null;
  onOpenPage(ref: TabRef): void;
}) {
  if (!panel) return null;
  return (
    <aside data-testid="tablet-pinned-panel" aria-label={panel.panel.title} style={asideStyle}>
      <div style={asideHead}>
        <span style={{ fontWeight: 600, flex: 1 }}>{panel.panel.title}</span>
        <OpenPageLink to={panel.open} name={panel.panel.title} onNavigate={onOpenPage} />
        <PinButton pinned onToggle={unpin} />
      </div>
      <PanelContent panel={panel} choose={choose} streamId={streamId} onOpenPage={onOpenPage} />
    </aside>
  );
}

function PanelContent({ panel, choose, streamId, onOpenPage }: { panel: TrayPanel; choose(panelId: string, param: string, value: string): void; streamId: string | null; onOpenPage(ref: TabRef): void }) {
  return (
    <>
      {panel.runs.choices.length > 0 ? (
        <div style={{ display: "flex", gap: 4, padding: "0 10px 6px" }}>
          {panel.runs.choices.map((c) => (
            <PanelChoiceToggle key={c.name} choice={c} testIdPrefix="tablet-panel" onPick={(v) => choose(panel.panel.id, c.name, v)} />
          ))}
        </div>
      ) : null}
      {panel.runs.body ? (
        <PanelBody body={panel.runs.body} filled testIdPrefix="tablet-panel" streamId={streamId} onOpenPage={onOpenPage} onMore={() => onOpenPage(panel.open)} />
      ) : (
        <div style={{ padding: "4px 10px", color: "var(--text-muted)" }}>Loading…</div>
      )}
    </>
  );
}

function PinButton({ pinned, onToggle }: { pinned: boolean; onToggle(): void }) {
  const Icon = pinned ? PinOff : Pin;
  const label = pinned ? "Unpin" : "Pin beside the page";
  return (
    <button type="button" className="ox-icon-button" aria-label={label} title={label} data-testid="tablet-panel-pin" onClick={onToggle}>
      <Icon size={16} aria-hidden />
    </button>
  );
}

const trayStyle: CSSProperties = { display: "flex", alignItems: "center", gap: "var(--space-2)", padding: "0 var(--space-2)", minHeight: "var(--hit-min)" };
const buttons: CSSProperties = { display: "flex", gap: 2, overflowX: "auto", minWidth: 0 };
const panelButton: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 6,
  minHeight: "var(--hit-min)",
  padding: "0 var(--space-2)",
  border: "none",
  borderRadius: 6,
  background: "transparent",
  color: "var(--text-secondary)",
  fontSize: "var(--text-sm)",
  flexShrink: 0,
};
const pinnedButton: CSSProperties = { color: "var(--text-primary)", background: "var(--surface-app)" };
const asideStyle: CSSProperties = { width: 320, flexShrink: 0, borderLeft: "1px solid var(--border-strong)", display: "flex", flexDirection: "column", minHeight: 0, background: "var(--surface-chrome)" };
const asideHead: CSSProperties = { display: "flex", alignItems: "center", gap: 4, padding: "0 var(--space-2)", minHeight: "var(--hit-min)" };
