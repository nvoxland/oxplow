import type { ReactNode } from "react";

export type DockSide = "left" | "right" | "bottom";

export interface ToolWindow {
  id: string;
  label: string;
  render: () => ReactNode;
}
