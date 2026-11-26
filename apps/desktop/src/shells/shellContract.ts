import type { ShellKind } from "./shellChoice.js";

/**
 * What every shell draws: the contributions the window has, whoever made
 * them (core or an extension). A shell that leaves one out hides a feature
 * on that device, so each is listed here with where each shell draws it.
 * `shellContract.test.tsx` checks the tablet shell draws what it claims; the
 * desktop layout's places are its existing components.
 */
export const SHELL_CONTRIBUTIONS = [
  /** The shown thread's tabs and their bodies (`WindowTab`, `TabBodies`). */
  "tabs",
  /** A tab's menu: its own entries, its commands, Close… (`buildTabMenu`). */
  "tab menus",
  /** The streams and threads, and their menus and flows (`useStreamThreadMenus`). */
  "streams and threads",
  /** Panels: body, collapsed form, badge, count, open, choices (`PanelParts`). */
  "panels",
  /** Dock panels (alerts, context, an extension's `dock: true`). */
  "dock panels",
  /** A firing panel badge as an alert and its toast. */
  "panel alerts",
  /** Commands with a `ui.menu` (the menu bar's), and Save, Find. */
  "menu commands",
  /** Search: Quick Open, Go To, the command list. */
  "search",
] as const;

export type ShellContribution = (typeof SHELL_CONTRIBUTIONS)[number];

/** Where each shell draws each contribution; `null` is not yet (a gap the
 *  plan fills, never a choice). */
export const SHELL_PLACES: Record<ShellKind, Record<ShellContribution, string | null>> = {
  desktop: {
    tabs: "CenterTabs",
    "tab menus": "CenterTabs (right-click)",
    "streams and threads": "Navigator",
    panels: "RailHud",
    "dock panels": "Dock",
    "panel alerts": "Alerts panel and toasts",
    "menu commands": "Menubar / the native menu",
    search: "TitleBar search → QuickOpenOverlay",
  },
  tablet: {
    tabs: "tablet-tabs",
    "tab menus": "tablet-tabs (long press)",
    "streams and threads": "tablet-context → TabletSwitcher",
    // The tray and sheet are after the switcher; the dock stands in.
    panels: null,
    "dock panels": "tablet-tray (the dock)",
    "panel alerts": "tablet-tray (the dock's Alerts) and toasts",
    "menu commands": "tablet-menu (File ▸, Edit ▸)",
    search: "tablet-search → QuickOpenOverlay",
  },
};
