/**
 * What a window tells the backend it shows (`report_tabs`): only what is
 * live — the thread it shows, its focus and visibility, the detail its
 * active page publishes and its page runs. The tabs themselves are the
 * stored ones (`tab_session`); an agent reads both with `oxplow_list_tabs`
 * and `oxplow_get_open_page` (`.context/pages-and-tabs.md` "What agents see
 * of the tabs").
 */
import type { TabReport } from "../tauri-bridge/generated/bindings.js";

export interface TabReportInput {
  /** All runs held anywhere in the window, not just its current tab. */
  runs?: readonly string[];
  focused: boolean;
  visible: boolean;
  shownThread: string | null;
  /** The shown thread's active page's published detail. */
  detail: Readonly<Record<string, unknown>> | null;
}

export function buildTabReport(input: TabReportInput): TabReport {
  return {
    runs: [...new Set(input.runs ?? [])].sort(),
    focused: input.focused,
    visible: input.visible,
    shown_thread: input.shownThread,
    detail_json: input.detail ? JSON.stringify(input.detail) : null,
  };
}

/** Reports acknowledge which runs the window still owns. Send them in
 *  order: an old HTTP reply must not overtake a newer roster and close its
 *  runs. While one is in flight only the latest pending snapshot matters. */
export function createTabReporter(write: (report: TabReport) => Promise<unknown>) {
  let pending: TabReport | null = null;
  let running = false;
  const drain = async () => {
    running = true;
    while (pending) {
      const report = pending;
      pending = null;
      try { await write(report); } catch { /* A later snapshot or heartbeat repairs it. */ }
    }
    running = false;
  };
  return {
    send(report: TabReport) {
      pending = report;
      if (!running) void drain();
    },
  };
}
