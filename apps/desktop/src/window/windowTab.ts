import type { ReactNode } from "react";
import type { AgentStatus } from "../api.js";
import type { MenuItem } from "../menu.js";

/** One tab of the shown thread, as every shell draws it: its strip entry
 *  (label, hint, status, menus) and its body (`render`). A hidden tab is a
 *  page in a tab's Back history, kept mounted so Back shows it as it was. */
export interface WindowTab {
  id: string;
  /** Its body's React key, when it must outlive a change of `id` (a page
   *  whose params changed stays mounted); `id` otherwise. */
  key?: string;
  label: string;
  /** What hovering the tab says, when it is more than the label (a file's
   *  whole path, where the label is its name). */
  hint?: string;
  closable: boolean;
  /** Pinned: it leads the strip, isn't dragged or dropped on, and the
   *  bulk closes ("Close Other Tabs") skip it (an agent session's tab). */
  pinned?: boolean;
  render: () => ReactNode;
  agentStatus?: AgentStatus;
  /** An agent session's harness's declared logo (`HarnessListing.icon`);
   *  the session's tab draws it in place of the generic robot. */
  icon?: string | null;
  /** What the agent is waiting on, when `agentStatus` is awaiting. */
  question?: string;
  /** Its page holds unsaved edits (`useTabEditing`): a ● before its
   *  label. */
  dirty?: boolean;
  /** Kind-specific right-click menu entries for this tab. They render at
   *  the top of the tab's context menu, above the universal "Close Other
   *  Tabs" / "Close Tabs to the Right" pair that the strip appends for
   *  every tab. (Right-click is the per-row action surface project-wide;
   *  there is no visible kebab — see `.context/usability.md`.) */
  contextMenu?: MenuItem[];
  /** Entries built as the menu opens, after `contextMenu` (what the
   *  window knows only then: Open In ▸ for a file's or a web page's tab). */
  menuAtOpen?: () => MenuItem[] | Promise<MenuItem[]>;
  /** When true, this tab does NOT appear in the strip but its body
   *  still mounts (kept hidden via display:none). Used by the host
   *  to keep back/forward stack entries alive so navigation between
   *  them preserves React component state without remounting. */
  hidden?: boolean;
}
