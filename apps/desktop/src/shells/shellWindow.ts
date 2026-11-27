import type { ReactNode } from "react";

import type { Stream, Thread, ThreadMark, ThreadState } from "../api.js";
import type { AgentStatusDotState } from "../components/AgentStatusDot.js";
import type { MenuItem } from "../menu.js";
import type { TabRef } from "../tabs/tabState.js";
import type { WindowTab } from "../window/windowTab.js";

/**
 * What a window gives the shell that draws it: its streams and threads, the
 * shown thread's tabs, and what a shell does with them. The window logic
 * (`App` and the `window/` hooks) makes it once; the desktop layout and the
 * tablet shell (`TabletShell`) each draw it their way. A shell draws every
 * contribution the window has (`shellContract.ts`).
 */
export interface ShellWindow {
  streams: Stream[];
  stream: Stream | null;
  /** Git is on in this workspace (the branch picker, new streams). */
  vcsEnabled: boolean;
  threadStates: Record<string, ThreadState>;
  selectedThread: Thread | null;
  streamStatuses: Record<string, AgentStatusDotState>;
  agentStatuses: Record<string, AgentStatusDotState>;
  agentQuestions: Record<string, string | undefined>;
  switchStream(id: string): void | Promise<void>;
  selectThread(streamId: string, threadId: string): void | Promise<void>;
  /** Whether it was made (a failure is the runner's: recorded, toasted). */
  createThread(streamId: string, title: string): Promise<boolean>;
  renameStream(streamId: string, title: string): void | Promise<void>;
  renameThread(threadId: string, title: string): void | Promise<void>;
  markThread(threadId: string, mark: ThreadMark | null): void | Promise<void>;
  closeThread(threadId: string): void | Promise<void>;
  openStreamSettings(streamId: string): void;
  openThreadSettings(threadId: string): void;
  openNewStream(): void;
  /** The shown thread's tabs, agent sessions first. */
  tabs: WindowTab[];
  activeTabId: string | null;
  activateTab(id: string): void;
  closeTab(id: string): void;
  /** The strip's new order (every tab id), as the desktop strip's drag gives it. */
  reorderTabs(orderedIds: string[]): void;
  openPage(ref: TabRef): void;
  /** Open Quick Open (search, Go To, the commands). */
  openSearch(): void;
  /** The menu bar's commands (Save, Find, the extensions' `ui.menu` ones)
   *  for a shell with no menu bar: File ▸, Edit ▸ (`shellMenu`). */
  menu: MenuItem[];
  /** Continue from <device>: one item per other device with pages open in
   *  the shown thread, read as the menu opens (`continueItems`). */
  continueFrom(): Promise<MenuItem[]>;
  /** The window's panels as a tray (`PanelTray`: the dock, holding the
   *  rail's panels and the status bar), for a shell with no rail. */
  tray: ReactNode;
  /** The panel the device pinned beside the page (`PinnedPanel`). */
  aside: ReactNode;
  /** Git ▸'s dialogs (Compare with…, Rollback…), docked over the page. */
  gitDialogs: ReactNode;
  /** What runs in the background (`StatusBar`): the desktop's dock holds
   *  it; the tablet's tray has its own. */
  statusBar: ReactNode;
}
