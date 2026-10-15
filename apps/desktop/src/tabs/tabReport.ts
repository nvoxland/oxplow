/**
 * What a window tells the backend it shows (`report_tabs`): every
 * thread's tabs with their shown titles, the one selected in each, the
 * thread the window shows, its focus and visibility, and the detail its
 * active page publishes. An agent reads it with `oxplow_list_tabs` and
 * `oxplow_get_open_page` (`.context/pages-and-tabs.md` "What agents see of
 * the tabs").
 */
import { pageKindLabel } from "../pageKinds.js";
import type { TabReport } from "../tauri-bridge/generated/bindings.js";
import type { TabRef } from "./tabState.js";

export interface TabReportInput {
  /** The window's renderer (`getUiClientId`). */
  client: string;
  focused: boolean;
  visible: boolean;
  shownThread: string | null;
  threadPageTabs: Readonly<Record<string, readonly TabRef[]>>;
  /** Each thread's selected tab id, as the window keeps it. */
  activeByThread: Readonly<Record<string, string | undefined>>;
  /** The title the window shows on a tab, when it has one. */
  titleOf: (tab: TabRef) => string | null | undefined;
  /** The shown thread's active page's published detail. */
  detail: Readonly<Record<string, unknown>> | null;
}

export function buildTabReport(input: TabReportInput): TabReport {
  return {
    client: input.client,
    focused: input.focused,
    visible: input.visible,
    shown_thread: input.shownThread,
    threads: Object.entries(input.threadPageTabs).map(([thread, tabs]) => {
      const active = input.activeByThread[thread];
      return {
        thread,
        tabs: tabs.map((tab) => ({ id: tab.id, kind: tab.kind, title: input.titleOf(tab) || pageKindLabel(tab.kind) })),
        active: active !== undefined && tabs.some((t) => t.id === active) ? active : null,
      };
    }),
    detail_json: input.detail ? JSON.stringify(input.detail) : null,
  };
}
