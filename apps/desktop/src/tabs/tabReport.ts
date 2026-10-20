/**
 * What a window tells the backend it shows (`report_tabs`): every
 * thread's tabs with their shown titles, where each one's history leads,
 * the one selected in each, the thread the window shows, its focus and
 * visibility, and the detail its active page publishes. An agent reads it
 * with `oxplow_list_tabs` and `oxplow_get_open_page`
 * (`.context/pages-and-tabs.md` "What agents see of the tabs").
 */
import { pageKindLabel } from "../pageKinds.js";
import type { TabReport } from "../tauri-bridge/generated/bindings.js";
import { activeRef, current, type Session } from "./session.js";
import type { TabRef } from "./tabState.js";

export interface TabReportInput {
  /** The window's renderer (`getUiClientId`). */
  client: string;
  focused: boolean;
  visible: boolean;
  shownThread: string | null;
  /** Every thread's tab session (`tabs/session.ts`). */
  sessions: Readonly<Record<string, Session>>;
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
    threads: Object.entries(input.sessions).map(([thread, session]) => ({
      thread,
      tabs: session.tabs.map((tab) => {
        const now = current(tab);
        return {
          id: now.ref.id,
          kind: now.ref.kind,
          title: input.titleOf(now.ref) || pageKindLabel(now.ref.kind),
          back: tab.entries.slice(0, tab.at).map((e) => e.ref.id),
          forward: tab.entries.slice(tab.at + 1).map((e) => e.ref.id),
          origin: now.from?.title ?? null,
        };
      }),
      active: activeRef(session)?.id ?? null,
    })),
    detail_json: input.detail ? JSON.stringify(input.detail) : null,
  };
}
