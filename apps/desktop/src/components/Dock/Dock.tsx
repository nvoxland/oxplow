/// The bottom row of the center column: the dock's icons at the left, what
/// the caller puts at the right (the status bar), and the open panel
/// rising above the row.

import type { ReactNode } from "react";

import type { TabRef } from "../../tabs/tabState.js";
import { DockBar } from "./DockBar.js";
import { DockPanel } from "./DockPanel.js";

export function Dock({ streamId, onOpenPage, children }: { streamId: string | null; onOpenPage(ref: TabRef): void; children?: ReactNode }) {
  return (
    // Gap above the bottom rail, matching the other panel gaps; the chrome
    // behind shows through.
    <div style={{ position: "relative", flexShrink: 0, marginTop: 6 }}>
      <DockPanel streamId={streamId} onOpenPage={onOpenPage} />
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: 8,
          padding: "4px 10px",
          background: "var(--bg-2)",
          minHeight: 26,
        }}
      >
        <DockBar />
        {children}
      </div>
    </div>
  );
}
