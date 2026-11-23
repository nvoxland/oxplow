/// View ▸ Left Rail's list (`menuBar.ts` `viewMenu`): the rail's panels as
/// the viewer's layout has them, and what shows or hides one — handed up to
/// the window that builds the menu bar. Draws nothing.
import { useEffect } from "react";

import type { RailPanelEntry } from "../../menuBar.js";
import { usePanelLayout } from "./PanelLayoutContext.js";

export interface RailMenu {
  panels: RailPanelEntry[];
  toggle(id: string): void;
}

export function RailMenuFeed({ onChange }: { onChange(menu: RailMenu): void }) {
  const { entries, toggle } = usePanelLayout();
  const key = JSON.stringify(entries);
  useEffect(() => {
    onChange({ panels: JSON.parse(key) as RailPanelEntry[], toggle });
  }, [key, toggle, onChange]);
  return null;
}
